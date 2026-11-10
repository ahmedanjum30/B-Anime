//! The local library: folders the user picked, scanned for video files that are matched to
//! AniList. Scans are incremental (unchanged files are skipped by size and modified time) and
//! user-confirmed matches are never overwritten.

pub mod parse;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::anilist::Client;
use crate::db::Db;
use crate::streaming::{similarity, title_score};
use crate::{Error, Result};
pub use parse::{Parsed, is_video, parse};

/// Folders that hold extras rather than episodes.
const SKIP_DIRS: &[&str] = &["extras", "extra", "bonus", "featurettes", "trailers", "ncop", "nced", "menus", "scans"];
const LOCAL_MATCH: f64 = 0.86;
const SEARCH_MATCH: f64 = 0.78;
/// Bumped when matching improves: files matched by an older version are matched again once
/// (hand-matched files never are).
const MATCHER_VERSION: i64 = 2;

// ── Library folders ──────────────────────────────────────────────────────────

pub async fn roots(db: &Db) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT path FROM library_root ORDER BY path").fetch_all(db).await?)
}

pub async fn add_root(db: &Db, path: &str) -> Result<String> {
    let p = PathBuf::from(path.trim());
    if !p.is_absolute() || !p.is_dir() {
        return Err(Error::Config(format!("{} isn't a folder", p.display())));
    }
    let clean = p.to_string_lossy().trim_end_matches(['\\', '/']).to_owned();
    let existing = roots(db).await?;
    let lower = clean.to_lowercase();
    if existing.iter().any(|r| {
        let r = r.to_lowercase();
        lower == r || lower.starts_with(&format!("{r}\\")) || lower.starts_with(&format!("{r}/"))
    }) {
        return Err(Error::Config("That folder is already in your library.".into()));
    }
    sqlx::query("INSERT INTO library_root (path) VALUES (?)").bind(&clean).execute(db).await?;
    Ok(clean)
}

pub async fn remove_root(db: &Db, path: &str) -> Result<()> {
    sqlx::query("DELETE FROM library_root WHERE path = ?").bind(path).execute(db).await?;
    let prefix = format!("{}%", path.trim_end_matches(['\\', '/']));
    sqlx::query("DELETE FROM local_file WHERE path LIKE ?").bind(prefix).execute(db).await?;
    Ok(())
}

// ── Scanning ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub files: usize,
    pub matched: usize,
    pub unmatched: usize,
    pub added: usize,
    pub removed: usize,
    pub millis: u128,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    /// "walking", "matching" or "saving".
    pub phase: &'static str,
    pub done: usize,
    pub total: usize,
}

struct Found {
    path: String,
    size: i64,
    modified: DateTime<Utc>,
}

fn walk(root: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy().to_lowercase();
        !(e.file_type().is_dir() && e.depth() > 0 && (name.starts_with('.') || SKIP_DIRS.contains(&name.as_str())))
    });
    for entry in walker.flatten() {
        if !entry.file_type().is_file() || !is_video(entry.path()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| Utc.timestamp_opt(d.as_secs() as i64, 0).single())
            .unwrap_or_else(Utc::now);
        out.push(Found { path: entry.path().to_string_lossy().into_owned(), size: meta.len() as i64, modified });
    }
    out
}

#[derive(sqlx::FromRow)]
struct Known {
    path: String,
    size_bytes: i64,
    modified_at: DateTime<Utc>,
    media_id: Option<i64>,
    locked: bool,
    ignored: bool,
}

/// Scans every library folder. Only new or changed files are parsed and matched.
pub async fn scan(db: &Db, client: &Client, progress: impl Fn(ScanProgress)) -> Result<ScanReport> {
    let started = Instant::now();
    let roots = roots(db).await?;

    progress(ScanProgress { phase: "walking", done: 0, total: 0 });
    let found: Vec<Found> = tokio::task::spawn_blocking(move || {
        roots.iter().filter(|r| Path::new(r).is_dir()).flat_map(|r| walk(Path::new(r))).collect()
    })
    .await
    .map_err(|e| Error::Config(e.to_string()))?;

    let known: Vec<Known> =
        sqlx::query_as("SELECT path, size_bytes, modified_at, media_id, locked, ignored FROM local_file").fetch_all(db).await?;
    let known: HashMap<String, Known> = known.into_iter().map(|k| (k.path.clone(), k)).collect();

    let found_paths: HashSet<&str> = found.iter().map(|f| f.path.as_str()).collect();
    let removed: Vec<&String> = known.keys().filter(|p| !found_paths.contains(p.as_str())).collect();

    // A better matcher re-matches everything it didn't get from the user's own hand.
    let version: i64 = crate::db::scalar_text(db, "SELECT value FROM setting WHERE key = 'library.matcher_version'")
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let rematch = version < MATCHER_VERSION;
    if rematch {
        sqlx::query("DELETE FROM title_match").execute(db).await?;
    }

    // New or changed files (or unmatched ones, which get another chance), minus extras.
    let mut work: Vec<(Found, Parsed, Vec<String>)> = Vec::new();
    for f in found.iter() {
        if let Some(k) = known.get(&f.path) {
            let unchanged = k.size_bytes == f.size && k.modified_at.timestamp() == f.modified.timestamp();
            if (unchanged && (k.media_id.is_some() || k.ignored) && !rematch) || k.locked || (k.ignored && unchanged) {
                continue;
            }
        }
        let path = Path::new(&f.path);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let folder = path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let parsed = parse(&name, folder.as_deref());
        if parsed.extra || parsed.title.is_empty() {
            continue;
        }
        // The season folder's own title ("Gintama°") is more specific than the file's ("Gintama").
        let mut queries: Vec<String> = folder.as_deref().and_then(parse::folder_title).into_iter().collect();
        if !queries.iter().any(|q| q.eq_ignore_ascii_case(&parsed.query())) {
            queries.push(parsed.query());
        }
        work.push((Found { path: f.path.clone(), size: f.size, modified: f.modified }, parsed, queries));
    }

    // Match each distinct set of titles once.
    let group_key = |queries: &[String]| queries.join("\u{1f}").to_lowercase();
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, (_, _, q)) in work.iter().enumerate() {
        groups.entry(group_key(q)).or_default().push(i);
    }
    let total = groups.len();
    let mut matches: HashMap<String, Option<i64>> = HashMap::new();
    for (done, (key, idx)) in groups.iter().enumerate() {
        progress(ScanProgress { phase: "matching", done, total });
        let (_, parsed, queries) = &work[idx[0]];
        matches.insert(key.clone(), match_title(db, client, queries, parsed.year).await?);
    }

    // Episodes past the end of a season continue in its sequel (split cours, absolute numbers).
    let mut placed: Vec<(Option<i64>, Option<i64>)> = Vec::with_capacity(work.len());
    for (_, p, q) in &work {
        let media_id = matches.get(&group_key(q)).copied().flatten();
        let episode = p.episode.map(|e| e.floor() as i64).or(media_id.map(|_| 1));
        placed.push(match (media_id, episode) {
            (Some(m), Some(e)) => {
                let (m, e) = follow_sequels(db, client, m, e).await?;
                (Some(m), Some(e))
            }
            other => other,
        });
    }

    progress(ScanProgress { phase: "saving", done: 0, total: work.len() });
    let mut tx = db.begin().await?;
    for path in &removed {
        sqlx::query("DELETE FROM local_file WHERE path = ?").bind(path.as_str()).execute(&mut *tx).await?;
    }
    let now = Utc::now();
    for ((f, p, _), (media_id, episode)) in work.iter().zip(placed) {
        let locked = false;
        sqlx::query(
            "INSERT INTO local_file (path, media_id, episode, size_bytes, modified_at, matched_at, locked, parsed_title, parsed_season)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(path) DO UPDATE SET media_id = excluded.media_id, episode = excluded.episode,
               size_bytes = excluded.size_bytes, modified_at = excluded.modified_at, matched_at = excluded.matched_at,
               locked = excluded.locked, parsed_title = excluded.parsed_title, parsed_season = excluded.parsed_season",
        )
        .bind(&f.path)
        .bind(media_id)
        .bind(episode)
        .bind(f.size)
        .bind(f.modified)
        .bind(media_id.map(|_| now))
        .bind(locked)
        .bind(&p.title)
        .bind(p.season.map(i64::from))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    if rematch {
        crate::db::set_setting(db, "library.matcher_version", &MATCHER_VERSION.to_string()).await?;
    }

    let (matched, unmatched): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(media_id IS NOT NULL), 0), COALESCE(SUM(media_id IS NULL AND ignored = 0), 0) FROM local_file",
    )
    .fetch_one(db)
    .await?;
    let report = ScanReport {
        files: found.len(),
        matched: matched as usize,
        unmatched: unmatched as usize,
        added: work.iter().filter(|(f, _, _)| !known.contains_key(&f.path)).count(),
        removed: removed.len(),
        millis: started.elapsed().as_millis(),
        finished_at: Some(now),
    };
    crate::db::set_setting(db, "library.last_scan", &serde_json::to_string(&report).unwrap_or_default()).await?;
    tracing::info!(files = report.files, matched = report.matched, unmatched = report.unmatched, ms = report.millis as u64, "library scan finished");
    Ok(report)
}

pub async fn last_scan(db: &Db) -> Result<Option<ScanReport>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Stored {
        files: usize,
        matched: usize,
        unmatched: usize,
        added: usize,
        removed: usize,
        millis: u128,
        finished_at: Option<DateTime<Utc>>,
    }
    let raw = crate::db::scalar_text(db, "SELECT value FROM setting WHERE key = 'library.last_scan'").await?;
    Ok(raw.and_then(|r| serde_json::from_str::<Stored>(&r).ok()).map(|s| ScanReport {
        files: s.files,
        matched: s.matched,
        unmatched: s.unmatched,
        added: s.added,
        removed: s.removed,
        millis: s.millis,
        finished_at: s.finished_at,
    }))
}

// ── Matching ─────────────────────────────────────────────────────────────────

/// Tries each title in order (most specific first): the user's list and known shows first
/// (no network), then a cached AniList search.
async fn match_title(db: &Db, client: &Client, queries: &[String], year: Option<u32>) -> Result<Option<i64>> {
    let local: Vec<(i64, String, Option<String>)> =
        sqlx::query_as("SELECT id, title, title_english FROM media WHERE kind = 'ANIME'").fetch_all(db).await?;
    for query in queries {
        let best_local = local
            .iter()
            .map(|(id, t, e)| {
                let s = title_score(query, t).max(e.as_deref().map(|e| title_score(query, e)).unwrap_or(0.0));
                (s, *id)
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        // With a specific folder title to try, only an exact local title is decisive; otherwise
        // a close one is enough.
        if let Some((score, id)) = best_local {
            if score >= LOCAL_MATCH && (score >= 1.0 || queries.len() == 1) {
                return Ok(Some(id));
            }
        }
        if let Some(id) = search_cached(db, client, query, year).await? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

async fn search_cached(db: &Db, client: &Client, query: &str, year: Option<u32>) -> Result<Option<i64>> {
    let key = query.to_lowercase();
    let cached: Option<(Option<i64>, DateTime<Utc>)> =
        sqlx::query_as("SELECT media_id, checked_at FROM title_match WHERE query = ?").bind(&key).fetch_optional(db).await?;
    if let Some((id, at)) = cached {
        // Misses are retried after a week; AniList gains entries.
        if id.is_some() || Utc::now() - at < chrono::Duration::days(7) {
            return Ok(id);
        }
    }

    let found = search_anilist(db, client, query, year).await;
    let id = match found {
        Ok(id) => id,
        Err(Error::Network(e)) => {
            tracing::warn!(error = %e, query, "AniList search failed during scan");
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    sqlx::query(
        "INSERT INTO title_match (query, media_id, checked_at) VALUES (?, ?, ?)
         ON CONFLICT(query) DO UPDATE SET media_id = excluded.media_id, checked_at = excluded.checked_at",
    )
    .bind(&key)
    .bind(id)
    .bind(Utc::now())
    .execute(db)
    .await?;
    // Stay well under AniList's rate limit when many titles are new.
    tokio::time::sleep(Duration::from_millis(700)).await;
    Ok(id)
}

const SEARCH: &str = r#"
query ($search: String) {
  Page(perPage: 8) {
    media(search: $search, type: ANIME, sort: SEARCH_MATCH) {
      id format status episodes duration seasonYear synonyms bannerImage
      title { userPreferred romaji english }
      coverImage { extraLarge large color }
      nextAiringEpisode { episode }
    }
  }
}"#;

async fn search_anilist(db: &Db, client: &Client, query: &str, year: Option<u32>) -> Result<Option<i64>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Data {
        page: PageData,
    }
    #[derive(Deserialize)]
    struct PageData {
        media: Vec<serde_json::Value>,
    }
    let data: Data = client.query(SEARCH, json!({ "search": query })).await?;
    let s = |v: &serde_json::Value| v.as_str().map(str::to_owned);

    let mut best: Option<(f64, &serde_json::Value)> = None;
    for m in &data.page.media {
        // Official titles count fully; synonyms are fan shorthand ("Gintama Season 5" is used
        // for several entries), so they can't win on their own.
        let titles: Vec<String> = ["romaji", "english", "userPreferred"].iter().filter_map(|k| s(&m["title"][*k])).collect();
        let synonyms: Vec<String> = m["synonyms"].as_array().map(|a| a.iter().filter_map(s).collect()).unwrap_or_default();
        let mut score = titles
            .iter()
            .map(|n| title_score(query, n))
            .chain(synonyms.iter().map(|n| similarity(query, n) - 0.05))
            .fold(0.0, f64::max);
        if let (Some(y), Some(my)) = (year, m["seasonYear"].as_i64()) {
            if i64::from(y) == my {
                score += 0.05;
            }
        }
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, m));
        }
    }
    let Some((score, m)) = best else { return Ok(None) };
    if score < SEARCH_MATCH {
        return Ok(None);
    }
    let id = m["id"].as_i64().ok_or_else(|| Error::AniList("search result without id".into()))?;
    // Keep a media row so the file's show has a title and art even off-list.
    sqlx::query(
        "INSERT INTO media (id, kind, title, title_english, cover_url, cover_thumb, banner_url, color, format, status,
                            episodes, duration_min, next_episode)
         VALUES (?, 'ANIME', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(id)
    .bind(s(&m["title"]["userPreferred"]).unwrap_or_else(|| query.to_owned()))
    .bind(s(&m["title"]["english"]))
    .bind(s(&m["coverImage"]["extraLarge"]))
    .bind(s(&m["coverImage"]["large"]))
    .bind(s(&m["bannerImage"]))
    .bind(s(&m["coverImage"]["color"]))
    .bind(s(&m["format"]))
    .bind(s(&m["status"]))
    .bind(m["episodes"].as_i64())
    .bind(m["duration"].as_i64())
    .bind(m["nextAiringEpisode"]["episode"].as_i64())
    .execute(db)
    .await?;
    Ok(Some(id))
}

// ── Sequels: split cours and absolute numbering ──────────────────────────────

/// Moves an episode number past the end of a show into its sequel(s): episode 13 of a
/// 12-episode season is episode 1 of the next. Stops when a count is unknown.
async fn follow_sequels(db: &Db, client: &Client, media_id: i64, episode: i64) -> Result<(i64, i64)> {
    let (mut id, mut ep) = (media_id, episode);
    for _ in 0..8 {
        let total: Option<i64> =
            sqlx::query_scalar("SELECT episodes FROM media WHERE id = ?").bind(id).fetch_optional(db).await?.flatten();
        let Some(total) = total.filter(|t| *t > 0) else { break };
        if ep <= total {
            break;
        }
        let Some(next) = sequel_of(db, client, id).await? else { break };
        ep -= total;
        id = next;
    }
    Ok((id, ep))
}

/// The direct TV/ONA sequel of a show (cached; looked up on AniList once).
async fn sequel_of(db: &Db, client: &Client, id: i64) -> Result<Option<i64>> {
    let cached: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT sequel_id FROM media_sequel WHERE media_id = ?").bind(id).fetch_optional(db).await?;
    if let Some((sequel,)) = cached {
        return Ok(sequel);
    }
    const QUERY: &str = r#"
      query ($id: Int) {
        Media(id: $id) {
          relations {
            edges {
              relationType(version: 2)
              node {
                id type format status episodes duration bannerImage
                title { userPreferred english }
                coverImage { extraLarge large color }
              }
            }
          }
        }
      }"#;
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Data {
        media: serde_json::Value,
    }
    let data: Data = match client.query(QUERY, json!({ "id": id })).await {
        Ok(d) => d,
        Err(Error::Network(e)) => {
            tracing::warn!(error = %e, id, "couldn't look up a sequel");
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    tokio::time::sleep(Duration::from_millis(700)).await;
    let s = |v: &serde_json::Value| v.as_str().map(str::to_owned);
    let edges = data.media["relations"]["edges"].as_array().cloned().unwrap_or_default();
    let sequel = edges.iter().find(|e| {
        e["relationType"] == "SEQUEL"
            && e["node"]["type"] == "ANIME"
            && matches!(e["node"]["format"].as_str(), Some("TV" | "TV_SHORT" | "ONA"))
    });
    let sequel_id = sequel.and_then(|e| e["node"]["id"].as_i64());
    if let Some(e) = sequel {
        let n = &e["node"];
        sqlx::query(
            "INSERT INTO media (id, kind, title, title_english, cover_url, cover_thumb, banner_url, color, format, status,
                                episodes, duration_min)
             VALUES (?, 'ANIME', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET episodes = COALESCE(media.episodes, excluded.episodes)",
        )
        .bind(n["id"].as_i64())
        .bind(s(&n["title"]["userPreferred"]).unwrap_or_default())
        .bind(s(&n["title"]["english"]))
        .bind(s(&n["coverImage"]["extraLarge"]))
        .bind(s(&n["coverImage"]["large"]))
        .bind(s(&n["bannerImage"]))
        .bind(s(&n["coverImage"]["color"]))
        .bind(s(&n["format"]))
        .bind(s(&n["status"]))
        .bind(n["episodes"].as_i64())
        .bind(n["duration"].as_i64())
        .execute(db)
        .await?;
    }
    sqlx::query("INSERT OR REPLACE INTO media_sequel (media_id, sequel_id, checked_at) VALUES (?, ?, ?)")
        .bind(id)
        .bind(sequel_id)
        .bind(Utc::now())
        .execute(db)
        .await?;
    Ok(sequel_id)
}

// ── Unmatched files and manual fixes ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmatchedGroup {
    pub title: String,
    pub season: Option<i64>,
    pub files: Vec<UnmatchedFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmatchedFile {
    pub path: String,
    pub file_name: String,
    pub episode: Option<i64>,
}

pub async fn unmatched(db: &Db) -> Result<Vec<UnmatchedGroup>> {
    let rows: Vec<(String, Option<i64>, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT path, episode, parsed_title, parsed_season FROM local_file
         WHERE media_id IS NULL AND ignored = 0 ORDER BY parsed_title, episode",
    )
    .fetch_all(db)
    .await?;
    let mut groups: Vec<UnmatchedGroup> = Vec::new();
    for (path, episode, title, season) in rows {
        let title = title.unwrap_or_else(|| "Unknown".into());
        let file_name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        let file = UnmatchedFile { path, file_name, episode };
        match groups.iter_mut().find(|g| g.title == title && g.season == season) {
            Some(g) => g.files.push(file),
            None => groups.push(UnmatchedGroup { title, season, files: vec![file] }),
        }
    }
    Ok(groups)
}

/// Assigns files to a show by hand. Locked: later scans keep it.
pub async fn assign(db: &Db, paths: &[String], media_id: i64) -> Result<()> {
    let mut tx = db.begin().await?;
    for p in paths {
        sqlx::query("UPDATE local_file SET media_id = ?, locked = 1, ignored = 0, matched_at = ? WHERE path = ?")
            .bind(media_id)
            .bind(Utc::now())
            .bind(p)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Hides files from matching (not episodes, or not wanted).
pub async fn ignore(db: &Db, paths: &[String]) -> Result<()> {
    let mut tx = db.begin().await?;
    for p in paths {
        sqlx::query("UPDATE local_file SET media_id = NULL, ignored = 1, locked = 0 WHERE path = ?").bind(p).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The file for an episode of a show, if the library has it.
pub async fn episode_file(db: &Db, media_id: i64, episode: i64) -> Result<Option<String>> {
    Ok(sqlx::query_scalar("SELECT path FROM local_file WHERE media_id = ? AND episode = ? ORDER BY locked DESC, path LIMIT 1")
        .bind(media_id)
        .bind(episode)
        .fetch_optional(db)
        .await?)
}

/// Subtitle files next to a video that belong to it: `Show - 05.mkv` picks up `Show - 05.srt`,
/// `Show - 05.en.ass`, `Show - 05.English.vtt`. Returns (path, language label).
pub fn sidecar_subtitles(video: &Path) -> Vec<(PathBuf, String)> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str())) else {
        return vec![];
    };
    let stem_lower = stem.to_lowercase();
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !matches!(ext.as_str(), "srt" | "ass" | "ssa" | "vtt") {
            continue;
        }
        let Some(sub_stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let lower = sub_stem.to_lowercase();
        if lower == stem_lower {
            out.push((path, "Subtitles".to_owned()));
        } else if let Some(rest) = lower.strip_prefix(&format!("{stem_lower}.")) {
            let label = match rest {
                "en" | "eng" | "english" => "English".to_owned(),
                "ja" | "jp" | "jpn" | "japanese" => "Japanese".to_owned(),
                other => other.to_owned(),
            };
            out.push((path, label));
        }
    }
    out.sort();
    out
}

/// Episode numbers available locally for a show.
pub async fn episodes(db: &Db, media_id: i64) -> Result<Vec<i64>> {
    Ok(sqlx::query_scalar("SELECT DISTINCT episode FROM local_file WHERE media_id = ? AND episode IS NOT NULL ORDER BY episode")
        .bind(media_id)
        .fetch_all(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scan_matches_from_the_users_list_and_keeps_manual_matches() {
        let db = crate::db::open_memory().await.unwrap();
        sqlx::query("INSERT INTO media (id, kind, title, episodes) VALUES (1, 'ANIME', 'Sousou no Frieren', 28)")
            .execute(&db)
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!("banime-scan-{}", std::process::id()));
        let show = dir.join("Frieren");
        std::fs::create_dir_all(show.join("Extras")).unwrap();
        for name in ["[Group] Sousou no Frieren - 01 [1080p].mkv", "[Group] Sousou no Frieren - 02 [1080p].mkv", "notes.txt"] {
            std::fs::write(show.join(name), b"x").unwrap();
        }
        std::fs::write(show.join("Extras").join("Sousou no Frieren - NCOP.mkv"), b"x").unwrap();
        std::fs::write(show.join("[Group] Sousou no Frieren - NCED1.mkv"), b"x").unwrap();
        add_root(&db, dir.to_str().unwrap()).await.unwrap();
        assert!(add_root(&db, show.to_str().unwrap()).await.is_err(), "nested root refused");

        let client = Client::new(None).unwrap();
        let report = scan(&db, &client, |_| {}).await.unwrap();
        assert_eq!(report.files, 3, "3 videos found: txt skipped, Extras folder skipped");
        assert_eq!(report.matched, 2);
        assert_eq!(episodes(&db, 1).await.unwrap(), vec![1, 2]);
        assert!(episode_file(&db, 1, 2).await.unwrap().unwrap().ends_with("- 02 [1080p].mkv"));

        // A hand match survives rescans; a deleted file disappears.
        let ep2 = episode_file(&db, 1, 2).await.unwrap().unwrap();
        sqlx::query("INSERT INTO media (id, kind, title) VALUES (2, 'ANIME', 'Other Show')").execute(&db).await.unwrap();
        assign(&db, std::slice::from_ref(&ep2), 2).await.unwrap();
        std::fs::remove_file(show.join("[Group] Sousou no Frieren - 01 [1080p].mkv")).unwrap();
        let again = scan(&db, &client, |_| {}).await.unwrap();
        assert_eq!(again.removed, 1);
        assert_eq!(episodes(&db, 2).await.unwrap(), vec![2]);
        assert!(episodes(&db, 1).await.unwrap().is_empty());

        // Subtitles next to an episode are found by name.
        let ep2_path = Path::new(&ep2);
        let stem = ep2_path.file_stem().unwrap().to_str().unwrap().to_owned();
        std::fs::write(ep2_path.with_file_name(format!("{stem}.en.ass")), b"x").unwrap();
        std::fs::write(ep2_path.with_file_name(format!("{stem}.srt")), b"x").unwrap();
        std::fs::write(show.join("unrelated.srt"), b"x").unwrap();
        let subs = sidecar_subtitles(ep2_path);
        let labels: Vec<&str> = subs.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(subs.len(), 2, "{subs:?}");
        assert!(labels.contains(&"English") && labels.contains(&"Subtitles"));

        remove_root(&db, dir.to_str().unwrap()).await.unwrap();
        assert!(episodes(&db, 2).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
