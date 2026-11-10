//! Data for the Home screen: "Up next" (next unwatched episode per show) and
//! "This week" (airing schedule for shows on the Watching list).

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde::Serialize;

use crate::Result;
use crate::db::Db;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mark {
    /// Episode aired within the last 7 days.
    New,
    /// Last episode of the season.
    Final,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpNext {
    pub media_id: i64,
    pub title: String,
    pub cover_url: Option<String>,
    /// Wide art for the card; falls back to the cover.
    pub banner_url: Option<String>,
    pub color: Option<String>,
    pub episode: i64,
    pub total_episodes: Option<i64>,
    pub duration_min: Option<i64>,
    /// Seconds into the episode if it was started.
    pub resume_s: Option<i64>,
    /// Episodes watched / total, 0..=1.
    pub progress: f64,
    pub mark: Option<Mark>,
    /// Evidence of where the episode comes from: the file name, or none when it isn't in the library.
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiringSlot {
    pub media_id: i64,
    pub title: String,
    /// The show's key colour (AniList), used as its channel swatch.
    pub color: Option<String>,
    pub episode: i64,
    pub airs_at: DateTime<Utc>,
    pub watched: bool,
    pub mark: Option<Mark>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub anilist_connected: bool,
    pub last_synced: Option<DateTime<Utc>>,
    pub files_matched: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeData {
    pub up_next: Vec<UpNext>,
    pub week: Vec<AiringSlot>,
    pub sync: SyncStatus,
    pub anime_count: i64,
    pub manga_count: i64,
    /// True when the user has no data yet and we show a labelled sample.
    pub is_sample: bool,
}

#[derive(sqlx::FromRow)]
struct UpNextRow {
    media_id: i64,
    title: String,
    cover_url: Option<String>,
    banner_url: Option<String>,
    color: Option<String>,
    episodes: Option<i64>,
    duration_min: Option<i64>,
    progress: i64,
    path: Option<String>,
    position_s: Option<i64>,
    aired_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct AiringRow {
    media_id: i64,
    title: String,
    color: Option<String>,
    episode: i64,
    episodes: Option<i64>,
    airs_at: DateTime<Utc>,
    progress: i64,
}

pub async fn load(db: &Db) -> Result<HomeData> {
    let entries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM list_entry")
        .fetch_one(db)
        .await?;
    let connected: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM setting WHERE key = 'anilist.viewer')")
            .fetch_one(db)
            .await?;
    if entries == 0 && !connected {
        return Ok(sample(Utc::now()));
    }

    let now = Utc::now();
    // The next episode of each show being watched, as long as it has aired. Episodes in the
    // library come first.
    let rows: Vec<UpNextRow> = sqlx::query_as(
        r#"
        SELECT m.id AS media_id, m.title, m.cover_url, m.banner_url, m.color, m.episodes, m.duration_min,
               e.progress, MIN(f.path) AS path, p.position_s, a.airs_at AS aired_at
        FROM list_entry e
        JOIN media m          ON m.id = e.media_id
        LEFT JOIN local_file f ON f.media_id = e.media_id AND f.episode = e.progress + 1
        LEFT JOIN playback p   ON p.media_id = e.media_id AND p.episode = e.progress + 1
        LEFT JOIN airing a     ON a.media_id = e.media_id AND a.episode = e.progress + 1
        WHERE m.kind = 'ANIME'
          AND e.status IN ('CURRENT', 'REPEATING')
          AND (m.episodes IS NULL OR e.progress < m.episodes)
          AND (m.next_episode IS NULL OR e.progress + 1 < m.next_episode)
          AND (m.status IS NULL OR m.status <> 'NOT_YET_RELEASED')
        GROUP BY e.media_id
        ORDER BY (MIN(f.path) IS NULL), e.updated_at DESC
        LIMIT 12
        "#,
    )
    .fetch_all(db)
    .await?;

    let up_next = rows
        .into_iter()
        .map(|r| {
            let episode = r.progress + 1;
            let is_final = r.episodes == Some(episode);
            let is_new = r.aired_at.is_some_and(|t| t <= now && now - t < Duration::days(7));
            UpNext {
                media_id: r.media_id,
                title: r.title,
                cover_url: r.cover_url,
                banner_url: r.banner_url,
                color: r.color,
                episode,
                total_episodes: r.episodes,
                duration_min: r.duration_min,
                resume_s: r.position_s.filter(|s| *s > 0),
                progress: ratio(r.progress, r.episodes),
                mark: pick_mark(is_new, is_final),
                source: r.path.as_deref().map(file_name),
            }
        })
        .collect();

    let (week_start, week_end) = week_bounds(now);
    let airing: Vec<AiringRow> = sqlx::query_as(
        r#"
        SELECT m.id AS media_id, m.title, m.color, a.episode, m.episodes, a.airs_at, e.progress
        FROM airing a
        JOIN media m      ON m.id = a.media_id
        JOIN list_entry e ON e.media_id = a.media_id
        WHERE e.status IN ('CURRENT', 'REPEATING') AND a.airs_at >= ? AND a.airs_at < ?
        ORDER BY a.airs_at
        "#,
    )
    .bind(week_start)
    .bind(week_end)
    .fetch_all(db)
    .await?;

    let week = airing
        .into_iter()
        .map(|r| AiringSlot {
            watched: r.progress >= r.episode,
            mark: pick_mark(r.airs_at <= now && r.progress < r.episode, r.episodes == Some(r.episode)),
            media_id: r.media_id,
            title: r.title,
            color: r.color,
            episode: r.episode,
            airs_at: r.airs_at,
        })
        .collect();

    let anime_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM list_entry e JOIN media m ON m.id = e.media_id WHERE m.kind = 'ANIME'")
            .fetch_one(db)
            .await?;
    let manga_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM list_entry e JOIN media m ON m.id = e.media_id WHERE m.kind = 'MANGA'")
            .fetch_one(db)
            .await?;
    let files_matched: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_file WHERE media_id IS NOT NULL")
        .fetch_one(db)
        .await?;
    let last_synced: Option<String> = sqlx::query_scalar("SELECT value FROM setting WHERE key = 'anilist.last_synced'")
        .fetch_optional(db)
        .await?;

    Ok(HomeData {
        up_next,
        week,
        sync: SyncStatus {
            anilist_connected: connected,
            last_synced: last_synced.and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|t| t.with_timezone(&Utc)),
            files_matched,
        },
        anime_count,
        manga_count,
        is_sample: false,
    })
}

/// One mark per item at most: FINAL outranks NEW.
fn pick_mark(is_new: bool, is_final: bool) -> Option<Mark> {
    match (is_new, is_final) {
        (_, true) => Some(Mark::Final),
        (true, false) => Some(Mark::New),
        _ => None,
    }
}

fn ratio(done: i64, total: Option<i64>) -> f64 {
    match total {
        Some(t) if t > 0 => (done as f64 / t as f64).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Monday 00:00 UTC of the current week, and the following Monday.
fn week_bounds(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let monday = now.date_naive() - Duration::days(now.weekday().num_days_from_monday() as i64);
    let start = Utc.from_utc_datetime(&monday.and_hms_opt(0, 0, 0).expect("valid midnight"));
    (start, start + Duration::days(7))
}

/// Labelled sample shown before the user connects AniList and adds a library.
/// Times are relative to `now` so the week view is always populated.
pub fn sample(now: DateTime<Utc>) -> HomeData {
    // Sample times are authored in the user's local evening, like real listings.
    let local_now = now.with_timezone(&chrono::Local);
    let monday = local_now.date_naive() - Duration::days(local_now.weekday().num_days_from_monday() as i64);
    let at = |day: i64, h: u32, m: u32| {
        let naive = (monday + Duration::days(day)).and_hms_opt(h, m, 0).expect("valid time");
        chrono::Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|| Utc.from_utc_datetime(&naive))
    };
    let up = |id, title: &str, color: &str, ep, total, dur, resume, mark, file: &str| UpNext {
        media_id: id,
        title: title.into(),
        cover_url: None,
        banner_url: None,
        color: Some(color.into()),
        episode: ep,
        total_episodes: total,
        duration_min: Some(dur),
        resume_s: resume,
        progress: ratio(ep - 1, total),
        mark,
        source: Some(file.into()),
    };
    let slot = |id, title: &str, color: &str, ep, airs_at: DateTime<Utc>, watched, mark| AiringSlot {
        media_id: id,
        title: title.into(),
        color: Some(color.into()),
        episode: ep,
        airs_at,
        watched,
        mark,
    };

    HomeData {
        up_next: vec![
            up(1, "Frieren: Beyond Journey's End", "#3d5a6e", 18, Some(28), 24, None, Some(Mark::New), "Frieren S2 - 18 [1080p].mkv"),
            up(2, "Dandadan", "#6e3d3d", 9, Some(12), 24, Some(842), None, "Dandadan S02E09.mkv"),
            up(3, "The Apothecary Diaries", "#4a4a6e", 12, Some(12), 24, None, Some(Mark::Final), "Kusuriya no Hitorigoto - 36.mkv"),
            up(4, "Mushishi", "#3d6e55", 3, Some(26), 25, None, None, "Mushishi - 03 (BD 1080p).mkv"),
        ],
        week: vec![
            slot(5, "Kaiju No. 8", "#8a7440", 7, at(0, 22, 30), true, None),
            slot(2, "Dandadan", "#a04a4a", 9, at(1, 17, 0), true, None),
            slot(3, "The Apothecary Diaries", "#6a64a8", 12, at(2, 22, 30), false, Some(Mark::Final)),
            slot(1, "Frieren: Beyond Journey's End", "#4f7c99", 19, at(4, 17, 0), false, Some(Mark::New)),
            slot(6, "Orb: On the Movements of the Earth", "#4f8f7a", 6, at(5, 22, 30), false, None),
            slot(7, "Blue Lock", "#4a6aa0", 21, at(3, 23, 30), false, None),
            slot(8, "Sakamoto Days", "#a0784a", 4, at(6, 23, 30), false, None),
        ],
        sync: SyncStatus {
            anilist_connected: false,
            last_synced: None,
            files_matched: 0,
        },
        anime_count: 0,
        manga_count: 0,
        is_sample: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_db_returns_labelled_sample() {
        let db = crate::db::open_memory().await.unwrap();
        let home = load(&db).await.unwrap();
        assert!(home.is_sample);
        assert!(!home.up_next.is_empty());
    }

    #[tokio::test]
    async fn up_next_picks_the_next_local_episode() {
        let db = crate::db::open_memory().await.unwrap();
        sqlx::query("INSERT INTO media (id, kind, title, episodes) VALUES (10, 'ANIME', 'Test Show', 12)")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO list_entry (media_id, status, progress) VALUES (10, 'CURRENT', 11)")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO local_file (path, media_id, episode, size_bytes, modified_at) VALUES ('D:/Anime/Test Show - 12.mkv', 10, 12, 1, '2026-01-01')")
            .execute(&db).await.unwrap();

        let home = load(&db).await.unwrap();
        assert!(!home.is_sample);
        assert_eq!(home.up_next.len(), 1);
        let u = &home.up_next[0];
        assert_eq!(u.episode, 12);
        assert_eq!(u.mark, Some(Mark::Final));
        assert_eq!(u.source.as_deref(), Some("Test Show - 12.mkv"));
    }

    #[tokio::test]
    async fn up_next_skips_unaired_and_lists_missing_files_last() {
        let db = crate::db::open_memory().await.unwrap();
        for sql in [
            // Airing: next to air is ep 5, user is at 4 -> nothing aired to watch.
            "INSERT INTO media (id, kind, title, status, next_episode) VALUES (1, 'ANIME', 'Caught Up', 'RELEASING', 5)",
            "INSERT INTO list_entry (media_id, status, progress, updated_at) VALUES (1, 'CURRENT', 4, '2026-01-03')",
            // Finished show, ep 3 not in library.
            "INSERT INTO media (id, kind, title, status, episodes) VALUES (2, 'ANIME', 'No File', 'FINISHED', 12)",
            "INSERT INTO list_entry (media_id, status, progress, updated_at) VALUES (2, 'CURRENT', 2, '2026-01-02')",
            // Finished show, ep 6 in library (older update, still first).
            "INSERT INTO media (id, kind, title, status, episodes) VALUES (3, 'ANIME', 'Has File', 'FINISHED', 12)",
            "INSERT INTO list_entry (media_id, status, progress, updated_at) VALUES (3, 'CURRENT', 5, '2026-01-01')",
            "INSERT INTO local_file (path, media_id, episode, size_bytes, modified_at) VALUES ('D:/a/Has File - 06.mkv', 3, 6, 1, '2026-01-01')",
        ] {
            sqlx::query(sql).execute(&db).await.unwrap();
        }
        let home = load(&db).await.unwrap();
        let titles: Vec<_> = home.up_next.iter().map(|u| u.title.as_str()).collect();
        assert_eq!(titles, ["Has File", "No File"]);
        assert_eq!(home.up_next[1].source, None);
    }

    #[test]
    fn final_outranks_new() {
        assert_eq!(pick_mark(true, true), Some(Mark::Final));
        assert_eq!(pick_mark(false, false), None);
    }
}
