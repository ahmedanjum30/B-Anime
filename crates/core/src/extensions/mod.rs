//! Extensions: third-party JavaScript providers for online streaming and manga sources.
//!
//! An extension is a manifest (JSON) plus one script defining `class Provider`. B-Anime ships
//! none; users install them from a URL or from repositories they add. See docs/extensions.md.

pub mod runtime;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Db;
use crate::{Error, Result};
pub use runtime::{CallOutput, Limits};

pub const API_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_CODE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtKind {
    Anime,
    Manga,
}

impl ExtKind {
    /// Methods a provider of this kind must have.
    pub fn required_methods(self) -> &'static [&'static str] {
        match self {
            ExtKind::Anime => &["search", "episodes", "sources"],
            ExtKind::Manga => &["search", "chapters", "pages"],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: ExtKind,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Where the script lives; relative URLs resolve against the manifest's URL.
    pub code_url: String,
    pub api_version: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// The manifest URL it was installed from; updates are checked there.
    pub source_url: String,
    pub enabled: bool,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Valid ids: lowercase letters, digits and dashes, 2–48 characters.
fn valid_id(id: &str) -> bool {
    (2..=48).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

/// Numeric-aware version comparison: "1.10.0" > "1.9.2".
pub fn newer(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split(|c: char| !c.is_ascii_digit()).filter_map(|p| p.parse().ok()).collect() };
    parts(candidate) > parts(current)
}

async fn download(url: &reqwest::Url, cap: usize, limits: Limits) -> Result<String> {
    runtime::check_url(url, limits.allow_private_hosts).map_err(Error::Extension)?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("B-Anime/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Network(e.to_string()))?;
    let mut res = client.get(url.clone()).send().await.map_err(|e| Error::Network(e.to_string()))?;
    if !res.status().is_success() {
        return Err(Error::Extension(format!("{url} answered HTTP {}", res.status().as_u16())));
    }
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| Error::Network(e.to_string()))? {
        if body.len() + chunk.len() > cap {
            return Err(Error::Extension(format!("{url} is larger than {} KB", cap / 1024)));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| Error::Extension(format!("{url} isn't UTF-8 text")))
}

/// Downloads and checks an extension without installing it.
pub async fn fetch(manifest_url: &str, limits: Limits) -> Result<(Manifest, String)> {
    let url = reqwest::Url::parse(manifest_url.trim()).map_err(|e| Error::Extension(format!("not a valid URL: {e}")))?;
    let text = download(&url, MAX_MANIFEST_BYTES, limits).await?;
    let manifest: Manifest =
        serde_json::from_str(&text).map_err(|e| Error::Extension(format!("the manifest isn't valid: {e}")))?;

    if !valid_id(&manifest.id) {
        return Err(Error::Extension(format!("invalid id {:?}: use lowercase letters, digits and dashes", manifest.id)));
    }
    if manifest.api_version != API_VERSION {
        return Err(Error::Extension(format!(
            "made for extension API v{}, this version of B-Anime supports v{API_VERSION}",
            manifest.api_version
        )));
    }
    let code_url = url
        .join(&manifest.code_url)
        .map_err(|e| Error::Extension(format!("bad codeUrl: {e}")))?;
    let code = download(&code_url, MAX_CODE_BYTES, limits).await?;

    // Load it in the sandbox and check it has what its kind needs.
    let probe = Limits { call_timeout: std::time::Duration::from_secs(10), ..limits };
    let methods: Vec<String> = serde_json::from_value(runtime::call(&code, "__methods", &Value::Array(vec![]), probe).await?.value)
        .map_err(|e| Error::Extension(e.to_string()))?;
    let missing: Vec<_> = manifest.kind.required_methods().iter().filter(|m| !methods.iter().any(|x| x == *m)).collect();
    if !missing.is_empty() {
        let list = missing.iter().map(|m| format!("{m}()")).collect::<Vec<_>>().join(", ");
        return Err(Error::Extension(format!("the Provider is missing {list}")));
    }
    Ok((manifest, code))
}

fn code_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.js"))
}

/// Installs (or replaces) an extension from its manifest URL.
pub async fn install(db: &Db, dir: &Path, manifest_url: &str, limits: Limits) -> Result<Installed> {
    let (manifest, code) = fetch(manifest_url, limits).await?;
    tokio::fs::create_dir_all(dir).await?;
    // Write then rename, so a half-written script is never loaded.
    let tmp = dir.join(format!("{}.js.part", manifest.id));
    tokio::fs::write(&tmp, &code).await?;
    tokio::fs::rename(&tmp, code_path(dir, &manifest.id)).await?;

    let json = serde_json::to_string(&manifest).map_err(|e| Error::Extension(e.to_string()))?;
    sqlx::query(
        "INSERT INTO extension (id, manifest, source_url) VALUES (?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET manifest = excluded.manifest, source_url = excluded.source_url,
           updated_at = datetime('now')",
    )
    .bind(&manifest.id)
    .bind(json)
    .bind(manifest_url.trim())
    .execute(db)
    .await?;
    get(db, &manifest.id).await?.ok_or_else(|| Error::Extension("install didn't stick".into()))
}

#[derive(sqlx::FromRow)]
struct Row {
    manifest: String,
    source_url: String,
    enabled: bool,
    installed_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Row {
    fn into_installed(self) -> Option<Installed> {
        Some(Installed {
            manifest: serde_json::from_str(&self.manifest).ok()?,
            source_url: self.source_url,
            enabled: self.enabled,
            installed_at: self.installed_at,
            updated_at: self.updated_at,
        })
    }
}

const SELECT: &str = "SELECT manifest, source_url, enabled, installed_at, updated_at FROM extension";

pub async fn list(db: &Db) -> Result<Vec<Installed>> {
    let rows: Vec<Row> = sqlx::query_as(SELECT).fetch_all(db).await?;
    let mut list: Vec<_> = rows.into_iter().filter_map(Row::into_installed).collect();
    list.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    Ok(list)
}

pub async fn get(db: &Db, id: &str) -> Result<Option<Installed>> {
    let row: Option<Row> = sqlx::query_as("SELECT manifest, source_url, enabled, installed_at, updated_at FROM extension WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.and_then(Row::into_installed))
}

pub async fn set_enabled(db: &Db, id: &str, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE extension SET enabled = ? WHERE id = ?").bind(enabled).bind(id).execute(db).await?;
    Ok(())
}

pub async fn remove(db: &Db, dir: &Path, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM extension WHERE id = ?").bind(id).execute(db).await?;
    match tokio::fs::remove_file(code_path(dir, id)).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// The version available at the extension's source, when it's newer than the installed one.
pub async fn available_update(ext: &Installed, limits: Limits) -> Result<Option<String>> {
    let url = reqwest::Url::parse(&ext.source_url).map_err(|e| Error::Extension(e.to_string()))?;
    let text = download(&url, MAX_MANIFEST_BYTES, limits).await?;
    let latest: Manifest = serde_json::from_str(&text).map_err(|e| Error::Extension(e.to_string()))?;
    Ok(newer(&latest.version, &ext.manifest.version).then_some(latest.version))
}

/// Runs a method of an installed, enabled extension.
pub async fn call(db: &Db, dir: &Path, id: &str, method: &str, args: &Value, limits: Limits) -> Result<CallOutput> {
    let ext = get(db, id).await?.ok_or_else(|| Error::Extension(format!("{id} isn't installed")))?;
    if !ext.enabled {
        return Err(Error::Extension(format!("{} is turned off", ext.manifest.name)));
    }
    let code = tokio::fs::read_to_string(code_path(dir, id)).await?;
    runtime::call(&code, method, args, limits).await
}

/// A repository: a JSON list of manifest URLs, or of manifests that carry a `manifestUrl`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoEntry {
    pub manifest_url: String,
    pub id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub kind: Option<ExtKind>,
    pub author: Option<String>,
    pub description: Option<String>,
}

pub async fn read_repo(url: &str, limits: Limits) -> Result<Vec<RepoEntry>> {
    let base = reqwest::Url::parse(url.trim()).map_err(|e| Error::Extension(format!("not a valid URL: {e}")))?;
    let text = download(&base, 1024 * 1024, limits).await?;
    let items: Vec<Value> = serde_json::from_str(&text).map_err(|e| Error::Extension(format!("not a repository list: {e}")))?;
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(items
        .into_iter()
        .filter_map(|v| {
            let raw = v.as_str().map(str::to_owned).or_else(|| s(&v, "manifestUrl"))?;
            let manifest_url = base.join(&raw).ok()?.to_string();
            Some(RepoEntry {
                id: s(&v, "id"),
                name: s(&v, "name").unwrap_or_else(|| manifest_url.clone()),
                version: s(&v, "version"),
                kind: v.get("kind").and_then(|k| serde_json::from_value(k.clone()).ok()),
                author: s(&v, "author"),
                description: s(&v, "description"),
                manifest_url,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get as route};

    #[tokio::test]
    async fn the_documented_example_works() {
        let code = include_str!("../../../../docs/examples/demo-streams/main.js");
        let manifest: Manifest =
            serde_json::from_str(include_str!("../../../../docs/examples/demo-streams/manifest.json")).unwrap();
        assert_eq!(manifest.kind, ExtKind::Anime);
        let limits = Limits::default();
        let found = runtime::call(code, "search", &serde_json::json!([{ "query": "anything" }]), limits).await.unwrap();
        let id = found.value[0]["id"].as_str().unwrap().to_owned();
        let eps = runtime::call(code, "episodes", &serde_json::json!([id]), limits).await.unwrap();
        let ep = eps.value[0]["id"].as_str().unwrap().to_owned();
        let src = runtime::call(code, "sources", &serde_json::json!([ep]), limits).await.unwrap();
        assert_eq!(src.value["sources"][0]["type"], "hls");
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("1.10.0", "1.9.2"));
        assert!(newer("2", "1.99"));
        assert!(!newer("1.0.0", "1.0.0"));
        assert!(!newer("0.9", "1.0"));
    }

    #[test]
    fn ids_are_strict() {
        assert!(valid_id("my-source-2"));
        assert!(!valid_id("../evil"));
        assert!(!valid_id("Caps"));
        assert!(!valid_id("-x"));
    }

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn test_limits() -> Limits {
        Limits { allow_private_hosts: true, ..Limits::default() }
    }

    #[tokio::test]
    async fn install_call_update_remove() {
        const CODE: &str = r#"class Provider {
            async search(q) { return [{ id: "a", title: "Result for " + q.query }]; }
            async episodes(id) { return [{ id: id + "-1", number: 1 }]; }
            async sources(ep) { return { sources: [{ url: "https://example.com/" + ep + ".m3u8", type: "hls" }] }; }
        }"#;
        let base = serve(
            Router::new()
                .route("/m.json", route(|| async {
                    r#"{"id":"test-source","name":"Test","version":"1.0.0","kind":"anime","codeUrl":"main.js","apiVersion":1}"#
                }))
                .route("/main.js", route(|| async { CODE }))
                .route("/repo.json", route(|| async { r#"["m.json", {"manifestUrl":"other.json","name":"Other"}]"# }))
                .route("/broken.json", route(|| async {
                    r#"{"id":"broken","name":"Broken","version":"1","kind":"manga","codeUrl":"main.js","apiVersion":1}"#
                })),
        )
        .await;

        let db = crate::db::open_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("banime-ext-test-{}", std::process::id()));

        let ext = install(&db, &dir, &format!("{base}/m.json"), test_limits()).await.unwrap();
        assert_eq!((ext.manifest.id.as_str(), ext.enabled), ("test-source", true));

        let out = call(&db, &dir, "test-source", "search", &serde_json::json!([{ "query": "x" }]), test_limits()).await.unwrap();
        assert_eq!(out.value[0]["title"], "Result for x");

        set_enabled(&db, "test-source", false).await.unwrap();
        assert!(call(&db, &dir, "test-source", "search", &serde_json::json!([{}]), test_limits()).await.is_err());

        assert_eq!(available_update(&ext, test_limits()).await.unwrap(), None);

        // A manga manifest pointing at an anime provider is refused.
        let err = install(&db, &dir, &format!("{base}/broken.json"), test_limits()).await.unwrap_err();
        assert!(err.to_string().contains("missing chapters(), pages()"), "{err}");

        let repo = read_repo(&format!("{base}/repo.json"), test_limits()).await.unwrap();
        assert_eq!(repo.len(), 2);
        assert_eq!(repo[1].manifest_url, format!("{base}/other.json"));

        remove(&db, &dir, "test-source").await.unwrap();
        assert!(list(&db).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
