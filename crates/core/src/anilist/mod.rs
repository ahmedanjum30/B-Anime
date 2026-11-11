//! AniList GraphQL client. AniList is the source of truth for lists and progress.

pub mod auth;
pub mod browse;
pub mod media;
pub mod sync;

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Error, Result};

const ENDPOINT: &str = "https://graphql.anilist.co";

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    token: Option<String>,
    endpoint: String,
}

impl Client {
    pub fn new(token: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("B-Anime/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Network(e.to_string()))?;
        Ok(Self { http, token, endpoint: ENDPOINT.to_owned() })
    }

    /// Tests only: talk to a stand-in GraphQL server instead of AniList.
    #[cfg(test)]
    pub fn with_endpoint(token: Option<String>, endpoint: &str) -> Result<Self> {
        Ok(Self { endpoint: endpoint.to_owned(), ..Self::new(token)? })
    }

    /// Runs a GraphQL query, waiting out AniList's rate limit (90/min) when it hits.
    pub async fn query<T: DeserializeOwned>(&self, query: &str, variables: Value) -> Result<T> {
        for attempt in 0..3 {
            let mut req = self.http.post(&self.endpoint).json(&json!({ "query": query, "variables": variables }));
            if let Some(token) = &self.token {
                req = req.bearer_auth(token);
            }
            let res = req.send().await.map_err(|e| Error::Network(e.to_string()))?;
            let status = res.status();

            if status.as_u16() == 429 && attempt < 2 {
                let wait = res
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(30);
                tracing::warn!(wait, "AniList rate limit hit; waiting");
                tokio::time::sleep(Duration::from_secs(wait.min(65))).await;
                continue;
            }
            if status.as_u16() == 401 {
                return Err(Error::Unauthorized);
            }

            let body: GqlResponse<T> = res.json().await.map_err(|e| Error::Network(e.to_string()))?;
            if let Some(errors) = body.errors.filter(|e| !e.is_empty()) {
                if errors.iter().any(|e| e.status == Some(401) || e.message.contains("Invalid token")) {
                    return Err(Error::Unauthorized);
                }
                let msg = errors.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
                return Err(Error::AniList(msg));
            }
            return body.data.ok_or_else(|| Error::AniList(format!("empty response (HTTP {status})")));
        }
        Err(Error::AniList("rate limited; try again in a minute".into()))
    }

    pub async fn viewer(&self) -> Result<Viewer> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Data {
            viewer: Viewer,
        }
        let d: Data = self
            .query("query { Viewer { id name avatar { medium } } }", json!({}))
            .await?;
        Ok(d.viewer)
    }
}

#[derive(Deserialize)]
struct GqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Deserialize)]
struct GqlError {
    message: String,
    status: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewer {
    pub id: i64,
    pub name: String,
    pub avatar: Option<Avatar>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Avatar {
    pub medium: Option<String>,
}
