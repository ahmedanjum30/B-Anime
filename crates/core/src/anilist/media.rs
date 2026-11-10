//! Show pages (full AniList detail) and list edits (SaveMediaListEntry).

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Client;
use crate::db::Db;
use crate::{Error, Result};

const DETAIL_QUERY: &str = r#"
query ($id: Int) {
  Media(id: $id) {
    id idMal type format status episodes chapters volumes duration source season seasonYear
    startDate { year month day }
    endDate { year month day }
    title { userPreferred romaji english native }
    description(asHtml: false)
    coverImage { extraLarge large color }
    bannerImage
    genres averageScore popularity siteUrl
    studios(isMain: true) { nodes { id name } }
    nextAiringEpisode { episode airingAt }
    streamingEpisodes { title thumbnail }
    relations {
      edges {
        relationType(version: 2)
        node { id type format status title { userPreferred } coverImage { large color } }
      }
    }
    recommendations(perPage: 10, sort: RATING_DESC) {
      nodes { mediaRecommendation { id type format title { userPreferred } coverImage { large color } } }
    }
  }
}"#;

const SAVE_MUTATION: &str = r#"
mutation ($mediaId: Int, $status: MediaListStatus, $progress: Int, $scoreRaw: Int) {
  SaveMediaListEntry(mediaId: $mediaId, status: $status, progress: $progress, scoreRaw: $scoreRaw) {
    status progress score(format: POINT_100) updatedAt
  }
}"#;

/// The show page payload: AniList's `Media` object as-is, plus what only we know.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPage {
    pub media: Value,
    /// The user's entry from the local mirror (kept current by syncs and edits).
    pub list_entry: Option<ListEntry>,
    /// Episodes found in the library: episode number and file name.
    pub local_files: Vec<LocalEpisode>,
    /// True when AniList couldn't be reached and this is the last saved copy.
    pub from_cache: bool,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ListEntry {
    pub status: String,
    pub progress: i64,
    pub score: Option<f64>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LocalEpisode {
    pub episode: i64,
    pub file_name: String,
}

/// Fresh from AniList when reachable, else the cached copy.
pub async fn page(client: &Client, db: &Db, id: i64) -> Result<MediaPage> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Data {
        media: Option<Value>,
    }

    let (media, from_cache, fetched_at) = match client.query::<Data>(DETAIL_QUERY, json!({ "id": id })).await {
        Ok(Data { media: Some(media) }) => {
            let now = Utc::now();
            sqlx::query(
                "INSERT INTO media_detail (media_id, json, fetched_at) VALUES (?, ?, ?)
                 ON CONFLICT(media_id) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
            )
            .bind(id)
            .bind(media.to_string())
            .bind(now)
            .execute(db)
            .await?;
            (media, false, now)
        }
        Ok(Data { media: None }) => return Err(Error::AniList(format!("no show with id {id}"))),
        Err(err @ (Error::Network(_) | Error::AniList(_))) => {
            let cached: Option<(String, DateTime<Utc>)> =
                sqlx::query_as("SELECT json, fetched_at FROM media_detail WHERE media_id = ?")
                    .bind(id)
                    .fetch_optional(db)
                    .await?;
            match cached {
                Some((json, at)) => (serde_json::from_str(&json).map_err(|e| Error::AniList(e.to_string()))?, true, at),
                None => return Err(err),
            }
        }
        Err(e) => return Err(e),
    };

    Ok(MediaPage {
        media,
        list_entry: list_entry(db, id).await?,
        local_files: local_files(db, id).await?,
        from_cache,
        fetched_at,
    })
}

async fn list_entry(db: &Db, id: i64) -> Result<Option<ListEntry>> {
    Ok(sqlx::query_as("SELECT status, progress, score, updated_at FROM list_entry WHERE media_id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?)
}

async fn local_files(db: &Db, id: i64) -> Result<Vec<LocalEpisode>> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT episode, path FROM local_file WHERE media_id = ? AND episode IS NOT NULL ORDER BY episode",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(episode, path)| LocalEpisode {
            episode,
            file_name: std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(path),
        })
        .collect())
}

/// What to change; fields left `None` stay as they are on AniList.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryChange {
    pub status: Option<String>,
    pub progress: Option<i64>,
    /// 0-100.
    pub score: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    status: String,
    progress: Option<i64>,
    score: Option<f64>,
    updated_at: Option<i64>,
}

/// Saves to AniList, then mirrors the result locally. Returns the entry as AniList now has it.
pub async fn save_entry(client: &Client, db: &Db, media_id: i64, change: EntryChange) -> Result<ListEntry> {
    const STATUSES: [&str; 6] = ["CURRENT", "PLANNING", "COMPLETED", "DROPPED", "PAUSED", "REPEATING"];
    if let Some(s) = &change.status {
        if !STATUSES.contains(&s.as_str()) {
            return Err(Error::AniList(format!("unknown list status {s}")));
        }
    }

    let mut vars = Map::new();
    vars.insert("mediaId".into(), json!(media_id));
    if let Some(s) = &change.status {
        vars.insert("status".into(), json!(s));
    }
    if let Some(p) = change.progress {
        vars.insert("progress".into(), json!(p.max(0)));
    }
    if let Some(s) = change.score {
        vars.insert("scoreRaw".into(), json!(s.clamp(0.0, 100.0).round() as i64));
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Data {
        save_media_list_entry: Saved,
    }
    let saved = client.query::<Data>(SAVE_MUTATION, Value::Object(vars)).await?.save_media_list_entry;

    ensure_media_row(db, media_id).await?;
    let entry = ListEntry {
        status: saved.status,
        progress: saved.progress.unwrap_or(0),
        score: saved.score.filter(|s| *s > 0.0),
        updated_at: saved
            .updated_at
            .and_then(|t| Utc.timestamp_opt(t, 0).single())
            .unwrap_or_else(Utc::now),
    };
    sqlx::query(
        "INSERT INTO list_entry (media_id, status, progress, score, updated_at) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(media_id) DO UPDATE SET status = excluded.status, progress = excluded.progress,
           score = excluded.score, updated_at = excluded.updated_at",
    )
    .bind(media_id)
    .bind(&entry.status)
    .bind(entry.progress)
    .bind(entry.score)
    .bind(entry.updated_at)
    .execute(db)
    .await?;
    Ok(entry)
}

/// The change that records `episode` as watched (or chapter as read), with the usual list rules:
/// reaching the last one completes the show, starting a planned one moves it to Watching.
/// `None` when the user is already at or past it.
pub fn progress_change(entry: Option<&ListEntry>, episode: i64, total: Option<i64>) -> Option<EntryChange> {
    if entry.is_some_and(|e| e.progress >= episode) {
        return None;
    }
    let status = match (entry.map(|e| e.status.as_str()), total) {
        (_, Some(t)) if episode >= t => Some("COMPLETED".to_owned()),
        (None | Some("PLANNING"), _) => Some("CURRENT".to_owned()),
        _ => None,
    };
    Some(EntryChange { status, progress: Some(episode), score: None })
}

/// A show's title and episode count from the local cache.
pub async fn title_and_total(db: &Db, id: i64) -> Result<(Option<String>, Option<i64>)> {
    let row: Option<(String, Option<i64>)> = sqlx::query_as("SELECT title, episodes FROM media WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|(t, e)| (Some(t), e)).unwrap_or((None, None)))
}

/// The user's local list entry for a show.
pub async fn local_entry(db: &Db, id: i64) -> Result<Option<ListEntry>> {
    list_entry(db, id).await
}

/// A show added from its page may not be in `media` yet; fill it from the cached detail.
pub async fn ensure_media_row(db: &Db, id: i64) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM media WHERE id = ?)")
        .bind(id)
        .fetch_one(db)
        .await?;
    if exists {
        return Ok(());
    }
    let json: Option<String> = sqlx::query_scalar("SELECT json FROM media_detail WHERE media_id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    let m: Value = json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or(Value::Null);
    let s = |v: &Value| v.as_str().map(str::to_owned);
    let n = |v: &Value| v.as_i64();
    sqlx::query(
        "INSERT INTO media (id, kind, title, title_english, cover_url, cover_thumb, banner_url, color, format, status,
                            episodes, chapters, duration_min, next_episode)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(s(&m["type"]).unwrap_or_else(|| "ANIME".into()))
    .bind(s(&m["title"]["userPreferred"]).unwrap_or_else(|| format!("#{id}")))
    .bind(s(&m["title"]["english"]))
    .bind(s(&m["coverImage"]["extraLarge"]))
    .bind(s(&m["coverImage"]["large"]))
    .bind(s(&m["bannerImage"]))
    .bind(s(&m["coverImage"]["color"]))
    .bind(s(&m["format"]))
    .bind(s(&m["status"]))
    .bind(n(&m["episodes"]))
    .bind(n(&m["chapters"]))
    .bind(n(&m["duration"]))
    .bind(n(&m["nextAiringEpisode"]["episode"]))
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_rules() {
        let at = |status: &str, progress| ListEntry { status: status.into(), progress, score: None, updated_at: Utc::now() };
        assert!(progress_change(Some(&at("CURRENT", 5)), 5, Some(12)).is_none());
        let c = progress_change(Some(&at("CURRENT", 4)), 5, Some(12)).unwrap();
        assert_eq!((c.status, c.progress), (None, Some(5)));
        let c = progress_change(Some(&at("CURRENT", 11)), 12, Some(12)).unwrap();
        assert_eq!(c.status.as_deref(), Some("COMPLETED"));
        let c = progress_change(Some(&at("PLANNING", 0)), 1, None).unwrap();
        assert_eq!(c.status.as_deref(), Some("CURRENT"));
        assert_eq!(progress_change(None, 1, Some(24)).unwrap().status.as_deref(), Some("CURRENT"));
    }

    #[tokio::test]
    async fn media_row_is_filled_from_cached_detail() {
        let db = crate::db::open_memory().await.unwrap();
        let detail = json!({
            "type": "ANIME", "format": "TV", "status": "FINISHED", "episodes": 12, "duration": 24,
            "title": { "userPreferred": "Show", "english": "The Show" },
            "coverImage": { "extraLarge": "x.jpg", "large": "l.jpg", "color": "#123456" }
        });
        sqlx::query("INSERT INTO media_detail (media_id, json, fetched_at) VALUES (5, ?, '2026-01-01T00:00:00Z')")
            .bind(detail.to_string())
            .execute(&db)
            .await
            .unwrap();
        ensure_media_row(&db, 5).await.unwrap();
        let (title, thumb, eps): (String, Option<String>, Option<i64>) =
            sqlx::query_as("SELECT title, cover_thumb, episodes FROM media WHERE id = 5")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!((title.as_str(), thumb.as_deref(), eps), ("Show", Some("l.jpg"), Some(12)));
    }
}
