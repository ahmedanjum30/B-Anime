//! The Library screen: every list entry of one kind (anime or manga). Sorting and filtering
//! happen in the UI, which holds the whole list; a few thousand rows is a small payload.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::Result;
use crate::db::Db;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Kind {
    Anime,
    Manga,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Anime => "ANIME",
            Kind::Manga => "MANGA",
        }
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub media_id: i64,
    pub title: String,
    pub title_english: Option<String>,
    /// Grid-sized cover, falling back to the full-size one.
    pub cover_url: Option<String>,
    pub color: Option<String>,
    pub format: Option<String>,
    /// AniList's release status: FINISHED, RELEASING, NOT_YET_RELEASED, ...
    pub media_status: Option<String>,
    /// Episodes (anime) or chapters (manga), when known.
    pub total: Option<i64>,
    /// Episodes out so far for a show still airing (next episode - 1).
    pub released: Option<i64>,
    /// The user's list status: CURRENT, PLANNING, COMPLETED, DROPPED, PAUSED, REPEATING.
    pub status: String,
    pub progress: i64,
    /// 0-100, none when unscored.
    pub score: Option<f64>,
    pub updated_at: DateTime<Utc>,
}

pub async fn list(db: &Db, kind: Kind) -> Result<Vec<LibraryEntry>> {
    let rows = sqlx::query_as(
        r#"
        SELECT m.id AS media_id, m.title, m.title_english,
               COALESCE(m.cover_thumb, m.cover_url) AS cover_url, m.color, m.format,
               m.status AS media_status,
               CASE WHEN m.kind = 'ANIME' THEN m.episodes ELSE m.chapters END AS total,
               CASE WHEN m.next_episode IS NOT NULL THEN m.next_episode - 1 END AS released,
               e.status, e.progress, e.score, e.updated_at
        FROM list_entry e
        JOIN media m ON m.id = e.media_id
        WHERE m.kind = ?
        ORDER BY e.updated_at DESC
        "#,
    )
    .bind(kind.as_str())
    .fetch_all(db)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lists_only_the_requested_kind_with_fallback_cover() {
        let db = crate::db::open_memory().await.unwrap();
        for sql in [
            "INSERT INTO media (id, kind, title, cover_url, episodes, next_episode) VALUES (1, 'ANIME', 'A', 'big.jpg', 12, 6)",
            "INSERT INTO media (id, kind, title, cover_url, cover_thumb, chapters) VALUES (2, 'MANGA', 'M', 'big.jpg', 'small.jpg', 100)",
            "INSERT INTO list_entry (media_id, status, progress) VALUES (1, 'CURRENT', 3)",
            "INSERT INTO list_entry (media_id, status, progress) VALUES (2, 'PLANNING', 0)",
        ] {
            sqlx::query(sql).execute(&db).await.unwrap();
        }
        let anime = list(&db, Kind::Anime).await.unwrap();
        assert_eq!(anime.len(), 1);
        assert_eq!(anime[0].released, Some(5));
        assert_eq!(anime[0].cover_url.as_deref(), Some("big.jpg"));

        let manga = list(&db, Kind::Manga).await.unwrap();
        assert_eq!(manga[0].total, Some(100));
        assert_eq!(manga[0].cover_url.as_deref(), Some("small.jpg"));
    }
}
