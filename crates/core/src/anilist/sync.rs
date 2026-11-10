//! Pulls the user's AniList lists and this week's airing schedule into SQLite.

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Client, Viewer};
use crate::db::Db;
use crate::{Error, Result};

const LIST_QUERY: &str = r#"
query ($userId: Int, $type: MediaType) {
  MediaListCollection(userId: $userId, type: $type, forceSingleCompletedList: true) {
    lists {
      entries {
        status
        progress
        score(format: POINT_100)
        updatedAt
        media {
          id
          type
          title { userPreferred english }
          coverImage { extraLarge large color }
          bannerImage
          format
          status
          episodes
          chapters
          duration
          nextAiringEpisode { episode airingAt }
        }
      }
    }
  }
}"#;

const AIRING_QUERY: &str = r#"
query ($ids: [Int], $from: Int, $to: Int, $page: Int) {
  Page(page: $page, perPage: 50) {
    pageInfo { hasNextPage }
    airingSchedules(mediaId_in: $ids, airingAt_greater: $from, airingAt_lesser: $to, sort: TIME) {
      mediaId
      episode
      airingAt
    }
  }
}"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub anime: usize,
    pub manga: usize,
    pub airing: usize,
    pub synced_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListData {
    media_list_collection: Option<Collection>,
}

#[derive(Deserialize)]
struct Collection {
    lists: Vec<List>,
}

#[derive(Deserialize)]
struct List {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    status: String,
    progress: Option<i64>,
    score: Option<f64>,
    updated_at: Option<i64>,
    media: Media,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Media {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
    title: Title,
    cover_image: Option<Cover>,
    banner_image: Option<String>,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    chapters: Option<i64>,
    duration: Option<i64>,
    next_airing_episode: Option<NextAiring>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Title {
    user_preferred: Option<String>,
    english: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cover {
    extra_large: Option<String>,
    large: Option<String>,
    color: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NextAiring {
    episode: i64,
    airing_at: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AiringData {
    page: AiringPage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiringPage {
    page_info: PageInfo,
    airing_schedules: Vec<Airing>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Airing {
    media_id: i64,
    episode: i64,
    airing_at: i64,
}

fn ts(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).single().unwrap_or_else(Utc::now)
}

/// Full sync: both lists replace the local mirror; airing covers last week through next week.
pub async fn sync_all(client: &Client, db: &Db, viewer: &Viewer) -> Result<SyncReport> {
    let mut counts = [0usize; 2];
    let mut airing_ids = Vec::new();
    let mut fetched = Vec::with_capacity(2);

    for (i, kind) in ["ANIME", "MANGA"].into_iter().enumerate() {
        let data: ListData = client
            .query(LIST_QUERY, json!({ "userId": viewer.id, "type": kind }))
            .await?;
        let entries: Vec<Entry> = data
            .media_list_collection
            .map(|c| c.lists.into_iter().flat_map(|l| l.entries).collect())
            .unwrap_or_default();
        counts[i] = entries.len();
        for e in &entries {
            if kind == "ANIME" && matches!(e.status.as_str(), "CURRENT" | "REPEATING" | "PLANNING") && e.media.status.as_deref() == Some("RELEASING") {
                airing_ids.push(e.media.id);
            }
        }
        fetched.push((kind, entries));
    }

    let now = Utc::now();
    // Last week through the end of next week, for Home and the Schedule screen.
    let from = now - Duration::days(8);
    let to = now + Duration::days(15);
    let mut airing = Vec::new();
    for chunk in airing_ids.chunks(50) {
        let mut page = 1;
        loop {
            let data: AiringData = client
                .query(
                    AIRING_QUERY,
                    json!({ "ids": chunk, "from": from.timestamp(), "to": to.timestamp(), "page": page }),
                )
                .await?;
            airing.extend(data.page.airing_schedules);
            if !data.page.page_info.has_next_page || page >= 10 {
                break;
            }
            page += 1;
        }
    }

    // One transaction: the UI never sees a half-synced list.
    let mut tx = db.begin().await?;
    for (kind, entries) in &fetched {
        sqlx::query("DELETE FROM list_entry WHERE media_id IN (SELECT id FROM media WHERE kind = ?)")
            .bind(kind)
            .execute(&mut *tx)
            .await?;
        for e in entries {
            let m = &e.media;
            let title = m.title.user_preferred.clone().or_else(|| m.title.english.clone()).unwrap_or_else(|| format!("#{}", m.id));
            let (cover, thumb, color) = m
                .cover_image
                .as_ref()
                .map(|c| (c.extra_large.clone(), c.large.clone(), c.color.clone()))
                .unwrap_or((None, None, None));
            sqlx::query(
                r#"INSERT INTO media (id, kind, title, title_english, cover_url, cover_thumb, banner_url, color, format, status,
                                      episodes, chapters, duration_min, next_episode, next_airing_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
                   ON CONFLICT(id) DO UPDATE SET
                     kind = excluded.kind, title = excluded.title, title_english = excluded.title_english,
                     cover_url = excluded.cover_url, cover_thumb = excluded.cover_thumb,
                     banner_url = excluded.banner_url, color = excluded.color,
                     format = excluded.format, status = excluded.status, episodes = excluded.episodes,
                     chapters = excluded.chapters, duration_min = excluded.duration_min,
                     next_episode = excluded.next_episode, next_airing_at = excluded.next_airing_at,
                     updated_at = excluded.updated_at"#,
            )
            .bind(m.id)
            .bind(&m.kind)
            .bind(&title)
            .bind(&m.title.english)
            .bind(cover)
            .bind(thumb)
            .bind(&m.banner_image)
            .bind(color)
            .bind(&m.format)
            .bind(&m.status)
            .bind(m.episodes)
            .bind(m.chapters)
            .bind(m.duration)
            .bind(m.next_airing_episode.as_ref().map(|n| n.episode))
            .bind(m.next_airing_episode.as_ref().map(|n| ts(n.airing_at)))
            .execute(&mut *tx)
            .await?;

            sqlx::query("INSERT INTO list_entry (media_id, status, progress, score, updated_at) VALUES (?, ?, ?, ?, ?)")
                .bind(m.id)
                .bind(&e.status)
                .bind(e.progress.unwrap_or(0))
                .bind(e.score.filter(|s| *s > 0.0))
                .bind(ts(e.updated_at.unwrap_or(0)))
                .execute(&mut *tx)
                .await?;
        }
    }

    sqlx::query("DELETE FROM airing").execute(&mut *tx).await?;
    for a in &airing {
        sqlx::query("INSERT OR REPLACE INTO airing (media_id, episode, airs_at) VALUES (?, ?, ?)")
            .bind(a.media_id)
            .bind(a.episode)
            .bind(ts(a.airing_at))
            .execute(&mut *tx)
            .await?;
    }

    let viewer_json = serde_json::to_string(viewer).map_err(|e| Error::AniList(e.to_string()))?;
    for (key, value) in [("anilist.viewer", viewer_json), ("anilist.last_synced", now.to_rfc3339())] {
        sqlx::query("INSERT INTO setting (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    tracing::info!(anime = counts[0], manga = counts[1], airing = airing.len(), "AniList sync finished");
    Ok(SyncReport { anime: counts[0], manga: counts[1], airing: airing.len(), synced_at: now })
}

/// The signed-in account as last synced, if any.
pub async fn stored_viewer(db: &Db) -> Result<Option<Viewer>> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM setting WHERE key = 'anilist.viewer'")
        .fetch_optional(db)
        .await?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

/// Forget the account locally: list mirror, airing and sync stamps. Media and files stay.
pub async fn forget(db: &Db) -> Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM list_entry").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM airing").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM setting WHERE key LIKE 'anilist.%'").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
