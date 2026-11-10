//! Search, Discover and Schedule: AniList browsing, annotated with the user's own list.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Client;
use crate::db::Db;
use crate::{Error, Result};

const MEDIA_FIELDS: &str = r#"
  id type format status episodes chapters seasonYear averageScore genres isAdult
  title { userPreferred }
  coverImage { large color }
  nextAiringEpisode { episode airingAt }
"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCard {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub color: Option<String>,
    pub format: Option<String>,
    pub media_status: Option<String>,
    /// Episodes (anime) or chapters (manga).
    pub total: Option<i64>,
    pub season_year: Option<i64>,
    pub average_score: Option<i64>,
    pub genres: Vec<String>,
    pub next_episode: Option<i64>,
    pub next_airing_at: Option<i64>,
    /// The user's list status, when the show is on their list.
    pub list_status: Option<String>,
    pub progress: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsePage {
    pub items: Vec<MediaCard>,
    pub has_next: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMedia {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    chapters: Option<i64>,
    season_year: Option<i64>,
    average_score: Option<i64>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    is_adult: bool,
    title: RawTitle,
    cover_image: Option<RawCover>,
    next_airing_episode: Option<RawNext>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTitle {
    user_preferred: Option<String>,
}

#[derive(Deserialize)]
struct RawCover {
    large: Option<String>,
    color: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNext {
    episode: i64,
    airing_at: i64,
}

impl RawMedia {
    fn into_card(self, list: &HashMap<i64, (String, i64)>) -> MediaCard {
        let entry = list.get(&self.id);
        let total = if self.kind == "MANGA" { self.chapters } else { self.episodes };
        MediaCard {
            title: self.title.user_preferred.unwrap_or_else(|| format!("#{}", self.id)),
            cover_url: self.cover_image.as_ref().and_then(|c| c.large.clone()),
            color: self.cover_image.as_ref().and_then(|c| c.color.clone()),
            next_episode: self.next_airing_episode.as_ref().map(|n| n.episode),
            next_airing_at: self.next_airing_episode.as_ref().map(|n| n.airing_at),
            list_status: entry.map(|e| e.0.clone()),
            progress: entry.map(|e| e.1),
            id: self.id,
            kind: self.kind,
            format: self.format,
            media_status: self.status,
            total,
            season_year: self.season_year,
            average_score: self.average_score,
            genres: self.genres,
        }
    }
}

/// The user's list as id -> (status, progress), to mark browse results.
async fn list_map(db: &Db) -> Result<HashMap<i64, (String, i64)>> {
    let rows: Vec<(i64, String, i64)> = sqlx::query_as("SELECT media_id, status, progress FROM list_entry")
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|(id, s, p)| (id, (s, p))).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Trending,
    ThisSeason,
    NextSeason,
    AllTime,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseFilter {
    /// Free-text search; when set, `section` is ignored and results sort by match.
    pub search: Option<String>,
    pub section: Option<Section>,
    pub kind: Option<String>,
    pub genre: Option<String>,
    pub format: Option<String>,
    pub year: Option<i64>,
    #[serde(default = "first_page")]
    pub page: i64,
}

fn first_page() -> i64 {
    1
}

/// AniList seasons: Dec–Feb is WINTER (of the year January falls in), then SPRING, SUMMER, FALL.
pub fn season_of(t: DateTime<Utc>) -> (&'static str, i32) {
    let (m, y) = (t.month(), t.year());
    match m {
        12 => ("WINTER", y + 1),
        1 | 2 => ("WINTER", y),
        3..=5 => ("SPRING", y),
        6..=8 => ("SUMMER", y),
        _ => ("FALL", y),
    }
}

fn next_season(season: &str, year: i32) -> (&'static str, i32) {
    match season {
        "WINTER" => ("SPRING", year),
        "SPRING" => ("SUMMER", year),
        "SUMMER" => ("FALL", year),
        _ => ("WINTER", year + 1),
    }
}

pub async fn browse(client: &Client, db: &Db, f: BrowseFilter) -> Result<BrowsePage> {
    let mut vars = Map::new();
    vars.insert("page".into(), json!(f.page.max(1)));
    vars.insert("type".into(), json!(f.kind.as_deref().unwrap_or("ANIME")));
    if let Some(g) = f.genre.as_deref().filter(|g| !g.is_empty()) {
        vars.insert("genre".into(), json!(g));
    }
    if let Some(fmt) = f.format.as_deref().filter(|s| !s.is_empty()) {
        vars.insert("format".into(), json!(fmt));
    }

    let search = f.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(q) = search {
        vars.insert("search".into(), json!(q));
        vars.insert("sort".into(), json!(["SEARCH_MATCH"]));
    } else {
        let (season, year) = season_of(Utc::now());
        match f.section.unwrap_or(Section::Trending) {
            Section::Trending => {
                vars.insert("sort".into(), json!(["TRENDING_DESC", "POPULARITY_DESC"]));
            }
            Section::ThisSeason => {
                vars.insert("season".into(), json!(season));
                vars.insert("seasonYear".into(), json!(year));
                vars.insert("sort".into(), json!(["POPULARITY_DESC"]));
            }
            Section::NextSeason => {
                let (s, y) = next_season(season, year);
                vars.insert("season".into(), json!(s));
                vars.insert("seasonYear".into(), json!(y));
                vars.insert("sort".into(), json!(["POPULARITY_DESC"]));
            }
            Section::AllTime => {
                vars.insert("sort".into(), json!(["POPULARITY_DESC"]));
            }
        }
        if let Some(y) = f.year {
            if !vars.contains_key("seasonYear") {
                vars.insert("year".into(), json!(format!("{y}%")));
            }
        }
    }

    let query = format!(
        r#"query ($page: Int, $type: MediaType, $search: String, $sort: [MediaSort], $season: MediaSeason,
                  $seasonYear: Int, $genre: String, $format: MediaFormat, $year: String) {{
          Page(page: $page, perPage: 30) {{
            pageInfo {{ hasNextPage }}
            media(type: $type, search: $search, sort: $sort, season: $season, seasonYear: $seasonYear,
                  genre: $genre, format: $format, startDate_like: $year, isAdult: false) {{ {MEDIA_FIELDS} }}
          }}
        }}"#
    );

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Data {
        page: PageData,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PageData {
        page_info: PageInfo,
        media: Vec<RawMedia>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PageInfo {
        has_next_page: bool,
    }

    let data: Data = client.query(&query, Value::Object(vars)).await?;
    let list = list_map(db).await?;
    Ok(BrowsePage {
        has_next: data.page.page_info.has_next_page,
        items: data.page.media.into_iter().filter(|m| !m.is_adult).map(|m| m.into_card(&list)).collect(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleItem {
    pub media_id: i64,
    pub title: String,
    pub cover_url: Option<String>,
    pub color: Option<String>,
    pub format: Option<String>,
    pub episode: i64,
    pub total_episodes: Option<i64>,
    pub airs_at: DateTime<Utc>,
    pub list_status: Option<String>,
    pub progress: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Shows on the user's Watching / Rewatching / Planning / Paused lists (local, works offline).
    Mine,
    /// Everything airing, from AniList.
    All,
}

pub async fn schedule(client: &Client, db: &Db, scope: Scope, from: i64, to: i64) -> Result<Vec<ScheduleItem>> {
    let from_t = Utc.timestamp_opt(from, 0).single().ok_or_else(|| Error::AniList("bad start time".into()))?;
    let to_t = Utc.timestamp_opt(to, 0).single().ok_or_else(|| Error::AniList("bad end time".into()))?;

    if scope == Scope::Mine {
        #[derive(sqlx::FromRow)]
        struct Row {
            media_id: i64,
            title: String,
            cover_url: Option<String>,
            color: Option<String>,
            format: Option<String>,
            episode: i64,
            episodes: Option<i64>,
            airs_at: DateTime<Utc>,
            status: String,
            progress: i64,
        }
        let rows: Vec<Row> = sqlx::query_as(
            r#"SELECT a.media_id, m.title, COALESCE(m.cover_thumb, m.cover_url) AS cover_url, m.color, m.format,
                      a.episode, m.episodes, a.airs_at, e.status, e.progress
               FROM airing a
               JOIN media m ON m.id = a.media_id
               JOIN list_entry e ON e.media_id = a.media_id
               WHERE e.status IN ('CURRENT', 'REPEATING', 'PLANNING', 'PAUSED') AND a.airs_at >= ? AND a.airs_at < ?
               ORDER BY a.airs_at"#,
        )
        .bind(from_t)
        .bind(to_t)
        .fetch_all(db)
        .await?;
        return Ok(rows
            .into_iter()
            .map(|r| ScheduleItem {
                media_id: r.media_id,
                title: r.title,
                cover_url: r.cover_url,
                color: r.color,
                format: r.format,
                episode: r.episode,
                total_episodes: r.episodes,
                airs_at: r.airs_at,
                list_status: Some(r.status),
                progress: Some(r.progress),
            })
            .collect());
    }

    const QUERY: &str = r#"
      query ($page: Int, $from: Int, $to: Int) {
        Page(page: $page, perPage: 50) {
          pageInfo { hasNextPage }
          airingSchedules(airingAt_greater: $from, airingAt_lesser: $to, sort: TIME) {
            episode airingAt
            media { id isAdult format episodes title { userPreferred } coverImage { large color } }
          }
        }
      }"#;
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Data {
        page: PageData,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PageData {
        page_info: Info,
        airing_schedules: Vec<Airing>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Info {
        has_next_page: bool,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Airing {
        episode: i64,
        airing_at: i64,
        media: Option<AiringMedia>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AiringMedia {
        id: i64,
        #[serde(default)]
        is_adult: bool,
        format: Option<String>,
        episodes: Option<i64>,
        title: RawTitle,
        cover_image: Option<RawCover>,
    }

    let list = list_map(db).await?;
    let mut items = Vec::new();
    for page in 1..=12 {
        let data: Data = client
            .query(QUERY, json!({ "page": page, "from": from - 1, "to": to }))
            .await?;
        for a in data.page.airing_schedules {
            let Some(m) = a.media.filter(|m| !m.is_adult) else { continue };
            let entry = list.get(&m.id);
            items.push(ScheduleItem {
                media_id: m.id,
                title: m.title.user_preferred.unwrap_or_else(|| format!("#{}", m.id)),
                cover_url: m.cover_image.as_ref().and_then(|c| c.large.clone()),
                color: m.cover_image.as_ref().and_then(|c| c.color.clone()),
                format: m.format,
                episode: a.episode,
                total_episodes: m.episodes,
                airs_at: Utc.timestamp_opt(a.airing_at, 0).single().unwrap_or(from_t),
                list_status: entry.map(|e| e.0.clone()),
                progress: entry.map(|e| e.1),
            });
        }
        if !data.page.page_info.has_next_page {
            break;
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasons_follow_anilist() {
        let at = |y, m| Utc.with_ymd_and_hms(y, m, 15, 0, 0, 0).unwrap();
        assert_eq!(season_of(at(2026, 12)), ("WINTER", 2027));
        assert_eq!(season_of(at(2027, 2)), ("WINTER", 2027));
        assert_eq!(season_of(at(2026, 4)), ("SPRING", 2026));
        assert_eq!(season_of(at(2026, 11)), ("FALL", 2026));
        assert_eq!(next_season("FALL", 2026), ("WINTER", 2027));
    }

    /// Hits the real AniList API: `cargo test -p banime-core -- --ignored live_`.
    #[tokio::test]
    #[ignore]
    async fn live_queries_are_valid() {
        let db = crate::db::open_memory().await.unwrap();
        let client = Client::new(None).unwrap();
        for section in [Section::Trending, Section::ThisSeason, Section::NextSeason, Section::AllTime] {
            let p = browse(&client, &db, BrowseFilter { section: Some(section), page: 1, ..Default::default() })
                .await
                .unwrap();
            assert!(!p.items.is_empty(), "{section:?} empty");
        }
        let filtered = browse(
            &client,
            &db,
            BrowseFilter {
                section: Some(Section::AllTime),
                genre: Some("Romance".into()),
                format: Some("MOVIE".into()),
                year: Some(2016),
                page: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(filtered.items.iter().all(|m| m.format.as_deref() == Some("MOVIE")));
        let found = browse(&client, &db, BrowseFilter { search: Some("frieren".into()), page: 1, ..Default::default() })
            .await
            .unwrap();
        assert!(found.items.iter().any(|m| m.title.to_lowercase().contains("frieren")));
        let now = Utc::now().timestamp();
        let all = schedule(&client, &db, Scope::All, now, now + 86400).await.unwrap();
        assert!(!all.is_empty());
        println!("trending ok, filtered {} movies, search {} hits, {} airing in 24h", filtered.items.len(), found.items.len(), all.len());
    }

    #[tokio::test]
    async fn my_schedule_comes_from_the_local_mirror() {
        let db = crate::db::open_memory().await.unwrap();
        for sql in [
            "INSERT INTO media (id, kind, title, episodes) VALUES (1, 'ANIME', 'Mine', 12)",
            "INSERT INTO media (id, kind, title) VALUES (2, 'ANIME', 'Dropped one')",
            "INSERT INTO list_entry (media_id, status, progress) VALUES (1, 'CURRENT', 3)",
            "INSERT INTO list_entry (media_id, status, progress) VALUES (2, 'DROPPED', 1)",
            "INSERT INTO airing (media_id, episode, airs_at) VALUES (1, 4, '2026-11-12T13:00:00Z')",
            "INSERT INTO airing (media_id, episode, airs_at) VALUES (2, 5, '2026-11-12T14:00:00Z')",
        ] {
            sqlx::query(sql).execute(&db).await.unwrap();
        }
        let client = Client::new(None).unwrap();
        let from = Utc.with_ymd_and_hms(2026, 11, 9, 0, 0, 0).unwrap().timestamp();
        let items = schedule(&client, &db, Scope::Mine, from, from + 7 * 86400).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].title.as_str(), items[0].episode, items[0].progress), ("Mine", 4, Some(3)));
    }
}
