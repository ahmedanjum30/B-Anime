//! Online streaming and reading through extensions: matching an AniList entry to a source,
//! listing its episodes/chapters, and turning sources into proxied URLs the UI can play.

pub mod proxy;

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db::Db;
use crate::extensions::{self, Limits};
use crate::{Error, Result};
pub use proxy::Proxy;

/// A search hit on a source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResult {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub year: Option<i64>,
}

/// An episode (anime) or chapter (manga) on a source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceItem {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub number: f64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub volume: Option<f64>,
}

fn string_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!("expected a string id, got {other}"))),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "status")]
pub enum MatchState {
    /// `auto` is false when the user picked it.
    Matched { source_id: String, title: String, auto: bool },
    /// No confident match: the best candidates, for the user to pick from.
    Unmatched { candidates: Vec<SourceResult> },
}

#[derive(Debug, Clone, Deserialize)]
struct RawStream {
    #[serde(default)]
    headers: HashMap<String, String>,
    sources: Vec<RawSource>,
    #[serde(default)]
    subtitles: Vec<RawSubtitle>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawSource {
    url: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawSubtitle {
    url: String,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPages {
    #[serde(default)]
    headers: HashMap<String, String>,
    pages: Vec<RawPage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawPage {
    Url(String),
    Obj { url: String },
}

/// Playable sources, already pointing at the local proxy.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    pub sources: Vec<StreamSource>,
    pub subtitles: Vec<StreamSubtitle>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSource {
    pub url: String,
    /// "hls" or "mp4".
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSubtitle {
    /// Always WebVTT by the time it gets here.
    pub url: String,
    pub lang: String,
    pub label: String,
}

fn parse<T: for<'de> Deserialize<'de>>(v: Value, what: &str) -> Result<T> {
    serde_json::from_value(v).map_err(|e| Error::Extension(format!("the extension returned unexpected {what}: {e}")))
}

// ── Matching ─────────────────────────────────────────────────────────────────

/// Lowercase alphanumeric words, with season markers normalised ("2nd season" -> "season 2").
fn normalize(s: &str) -> String {
    let lowered: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let words: Vec<&str> = lowered.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let ordinal = w.trim_end_matches(|c: char| c.is_alphabetic());
        if !ordinal.is_empty() && ordinal.len() < w.len() && words.get(i + 1) == Some(&"season") {
            out.push("season".into());
            out.push(ordinal.into());
            i += 2;
            continue;
        }
        out.push(w.into());
        i += 1;
    }
    out.join(" ")
}

/// Titles that differ only in spacing, colons, dashes, case or long-vowel spelling (ō / ou),
/// with `_` in either standing for a character Windows forbids in file names ("Gintama_" for
/// "Gintama."). Punctuation that tells seasons apart (Gintama' vs Gintama°) still counts.
pub fn same_title(a: &str, b: &str) -> bool {
    fn canon(s: &str) -> Vec<char> {
        let mut out = Vec::new();
        for c in s.trim().to_lowercase().chars() {
            match c {
                'ō' | 'ô' => out.extend(['o', 'u']),
                'ū' | 'û' => out.extend(['u', 'u']),
                'ā' | 'â' => out.extend(['a', 'a']),
                'ī' | 'î' => out.extend(['i', 'i']),
                'ē' | 'ê' => out.extend(['e', 'e']),
                c if c.is_whitespace() || c == ':' || c == '-' || c == '–' => {}
                c => out.push(c),
            }
        }
        out
    }
    let (a, b) = (canon(a), canon(b));
    a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x == y || ((*x == '_') ^ (*y == '_') && !(x.is_alphanumeric() && y.is_alphanumeric())))
}

/// Similarity plus a bonus for an exact title, so near-identical franchise entries
/// (Gintama, Gintama', Gintama°) resolve to the one actually named.
pub fn title_score(query: &str, candidate: &str) -> f64 {
    similarity(query, candidate) + if same_title(query, candidate) { 0.25 } else { 0.0 }
}

/// Dice coefficient over character bigrams of the normalised titles, 0..=1.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (normalize(a), normalize(b));
    if a == b {
        return 1.0;
    }
    let grams = |s: &str| -> Vec<(char, char)> {
        let c: Vec<char> = s.chars().collect();
        c.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let (ga, gb) = (grams(&a), grams(&b));
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    let mut pool = gb.clone();
    let mut hits = 0;
    for g in &ga {
        if let Some(i) = pool.iter().position(|x| x == g) {
            pool.swap_remove(i);
            hits += 1;
        }
    }
    2.0 * hits as f64 / (ga.len() + gb.len()) as f64
}

const AUTO_MATCH: f64 = 0.82;

async fn call(db: &Db, dir: &Path, ext: &str, method: &str, args: Value) -> Result<Value> {
    Ok(extensions::call(db, dir, ext, method, &args, Limits::default()).await?.value)
}

pub async fn search(db: &Db, dir: &Path, ext: &str, query: &str) -> Result<Vec<SourceResult>> {
    parse(call(db, dir, ext, "search", json!([{ "query": query }])).await?, "search results")
}

/// The saved match for this show on this source, or the best automatic one (saved when confident).
pub async fn find_match(db: &Db, dir: &Path, media_id: i64, ext: &str, titles: &[String], year: Option<i64>) -> Result<MatchState> {
    let saved: Option<(String, String, bool)> =
        sqlx::query_as("SELECT source_id, source_title, auto FROM source_match WHERE media_id = ? AND ext_id = ?")
            .bind(media_id)
            .bind(ext)
            .fetch_optional(db)
            .await?;
    if let Some((source_id, title, auto)) = saved {
        return Ok(MatchState::Matched { source_id, title, auto });
    }

    let mut scored: Vec<(f64, SourceResult)> = Vec::new();
    for query in titles.iter().filter(|t| !t.trim().is_empty()).take(2) {
        for r in search(db, dir, ext, query).await? {
            let mut score = titles.iter().map(|t| similarity(t, &r.title)).fold(0.0, f64::max);
            if let (Some(a), Some(b)) = (year, r.year) {
                if a == b {
                    score += 0.05;
                }
            }
            if !scored.iter().any(|(_, x)| x.id == r.id) {
                scored.push((score, r));
            }
        }
        if scored.iter().any(|(s, _)| *s >= 0.99) {
            break;
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    match scored.first() {
        Some((score, best)) if *score >= AUTO_MATCH => {
            save_match(db, media_id, ext, &best.id, &best.title, true).await?;
            Ok(MatchState::Matched { source_id: best.id.clone(), title: best.title.clone(), auto: true })
        }
        _ => Ok(MatchState::Unmatched { candidates: scored.into_iter().take(12).map(|(_, r)| r).collect() }),
    }
}

pub async fn save_match(db: &Db, media_id: i64, ext: &str, source_id: &str, title: &str, auto: bool) -> Result<()> {
    sqlx::query(
        "INSERT INTO source_match (media_id, ext_id, source_id, source_title, auto) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(media_id, ext_id) DO UPDATE SET source_id = excluded.source_id,
           source_title = excluded.source_title, auto = excluded.auto, updated_at = datetime('now')",
    )
    .bind(media_id)
    .bind(ext)
    .bind(source_id)
    .bind(title)
    .bind(auto)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn clear_match(db: &Db, media_id: i64, ext: &str) -> Result<()> {
    sqlx::query("DELETE FROM source_match WHERE media_id = ? AND ext_id = ?").bind(media_id).bind(ext).execute(db).await?;
    Ok(())
}

// ── Episodes, chapters, sources, pages ───────────────────────────────────────

/// Episodes (anime) or chapters (manga), sorted by number.
pub async fn items(db: &Db, dir: &Path, ext: &str, kind: extensions::ExtKind, source_id: &str) -> Result<Vec<SourceItem>> {
    let method = match kind {
        extensions::ExtKind::Anime => "episodes",
        extensions::ExtKind::Manga => "chapters",
    };
    let mut list: Vec<SourceItem> = parse(call(db, dir, ext, method, json!([source_id])).await?, method)?;
    list.retain(|i| i.number.is_finite() && i.number >= 0.0);
    list.sort_by(|a, b| a.number.total_cmp(&b.number));
    list.dedup_by(|a, b| a.number == b.number);
    Ok(list)
}

fn guess_kind(url: &str, declared: Option<&str>) -> String {
    match declared.map(str::to_ascii_lowercase).as_deref() {
        Some("hls") | Some("m3u8") => "hls".into(),
        Some("mp4") => "mp4".into(),
        _ if url.to_ascii_lowercase().contains(".m3u8") => "hls".into(),
        _ => "mp4".into(),
    }
}

fn sub_format(url: &str, declared: Option<&str>) -> &'static str {
    let probe = declared.map(str::to_ascii_lowercase).unwrap_or_else(|| url.to_ascii_lowercase());
    if probe.contains("ass") || probe.contains("ssa") {
        "ass"
    } else if probe.contains("srt") {
        "srt"
    } else {
        "vtt"
    }
}

pub async fn stream(db: &Db, dir: &Path, proxy: &Proxy, ext: &str, episode_id: &str) -> Result<Stream> {
    let raw: RawStream = parse(call(db, dir, ext, "sources", json!([episode_id])).await?, "sources")?;
    if raw.sources.is_empty() {
        return Err(Error::Extension("the source has no video for this episode".into()));
    }
    let session = proxy.session(&raw.headers);
    let mut sources = Vec::new();
    for (i, s) in raw.sources.iter().enumerate() {
        sources.push(StreamSource {
            url: proxy.url(&session, &s.url, None)?,
            kind: guess_kind(&s.url, s.kind.as_deref()),
            label: s.label.clone().or_else(|| s.quality.clone()).unwrap_or_else(|| format!("Source {}", i + 1)),
        });
    }
    let mut subtitles = Vec::new();
    for s in &raw.subtitles {
        let format = sub_format(&s.url, s.format.as_deref());
        let lang = s.lang.clone().unwrap_or_else(|| "und".into());
        subtitles.push(StreamSubtitle {
            url: proxy.url(&session, &s.url, Some(format))?,
            label: s.label.clone().unwrap_or_else(|| lang.clone()),
            lang,
        });
    }
    Ok(Stream { sources, subtitles })
}

pub async fn pages(db: &Db, dir: &Path, proxy: &Proxy, ext: &str, chapter_id: &str) -> Result<Vec<String>> {
    let raw: RawPages = parse(call(db, dir, ext, "pages", json!([chapter_id])).await?, "pages")?;
    let session = proxy.session(&raw.headers);
    raw.pages
        .iter()
        .map(|p| {
            let url = match p {
                RawPage::Url(u) | RawPage::Obj { url: u } => u,
            };
            proxy.url(&session, url, None)
        })
        .collect()
}

// ── Skip times (AniSkip) and playback positions ──────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipTime {
    /// "op" (intro), "ed" (outro), "recap", "mixed-op", "mixed-ed".
    pub kind: String,
    pub start: f64,
    pub end: f64,
}

/// Community intro/outro timestamps from api.aniskip.com (keyed by MyAnimeList id). Best effort.
pub async fn skip_times(mal_id: i64, episode: i64, episode_length: f64) -> Result<Vec<SkipTime>> {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        results: Vec<Item>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        interval: Interval,
        skip_type: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Interval {
        start_time: f64,
        end_time: f64,
    }
    let url = format!(
        "https://api.aniskip.com/v2/skip-times/{mal_id}/{episode}?types[]=op&types[]=ed&types[]=recap&types[]=mixed-op&types[]=mixed-ed&episodeLength={}",
        episode_length.max(0.0).round()
    );
    let client = reqwest::Client::builder()
        .user_agent(concat!("B-Anime/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| Error::Network(e.to_string()))?;
    let res = client.get(url).send().await.map_err(|e| Error::Network(e.to_string()))?;
    if res.status().as_u16() == 404 {
        return Ok(vec![]);
    }
    let body: Resp = res.json().await.map_err(|e| Error::Network(e.to_string()))?;
    Ok(body
        .results
        .into_iter()
        .map(|i| SkipTime { kind: i.skip_type, start: i.interval.start_time, end: i.interval.end_time })
        .collect())
}

pub async fn save_position(db: &Db, media_id: i64, episode: i64, position: f64, duration: f64) -> Result<()> {
    sqlx::query(
        "INSERT INTO playback (media_id, episode, position_s, duration_s, updated_at) VALUES (?, ?, ?, ?, datetime('now'))
         ON CONFLICT(media_id, episode) DO UPDATE SET position_s = excluded.position_s,
           duration_s = excluded.duration_s, updated_at = excluded.updated_at",
    )
    .bind(media_id)
    .bind(episode)
    .bind(position.max(0.0).round() as i64)
    .bind(duration.max(0.0).round() as i64)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn position(db: &Db, media_id: i64, episode: i64) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar("SELECT position_s FROM playback WHERE media_id = ? AND episode = ?")
        .bind(media_id)
        .bind(episode)
        .fetch_optional(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_match_across_spellings() {
        assert!(similarity("Sousou no Frieren", "Sousou no Frieren") > 0.99);
        assert!(similarity("Black Clover 2nd Season", "Black Clover Season 2") > 0.99);
        assert!(similarity("Kusuriya no Hitorigoto", "Kusuriya no Hitorigoto (TV)") >= AUTO_MATCH);
        assert!(similarity("Sousou no Frieren", "Big Buck Bunny") < 0.3);
        assert!(similarity("Dandadan", "Dandadan 2nd Season") < AUTO_MATCH);
    }

    #[test]
    fn exact_titles_beat_franchise_siblings() {
        assert!(same_title("Gintama°", "Gintama°"));
        assert!(same_title("Gintama_", "Gintama."));
        assert!(same_title("Gintama' Enchōsen", "Gintama': Enchousen"));
        assert!(same_title("Gintama. Porori-hen", "Gintama.: Porori-hen"));
        assert!(!same_title("Gintama'", "Gintama°"));
        assert!(!same_title("Gintama. Shirogane no Tamashii-hen", "Gintama.: Shirogane no Tamashii-hen 2"));
        assert!(!same_title("Gintama_", "Gintama"));
        let candidates = ["Gintama", "Gintama'", "Gintama°", "Gintama.", "Gintama': Enchousen"];
        let best = |q: &str| *candidates.iter().max_by(|a, b| title_score(q, a).total_cmp(&title_score(q, b))).unwrap();
        assert_eq!(best("Gintama°"), "Gintama°");
        assert_eq!(best("Gintama_"), "Gintama.");
        assert_eq!(best("Gintama' Enchōsen"), "Gintama': Enchousen");
    }

    #[test]
    fn match_state_serializes_for_the_ui() {
        let m = MatchState::Matched { source_id: "s".into(), title: "t".into(), auto: true };
        assert_eq!(
            serde_json::to_value(m).unwrap(),
            json!({ "status": "matched", "sourceId": "s", "title": "t", "auto": true })
        );
    }

    #[tokio::test]
    async fn manual_match_is_remembered() {
        let db = crate::db::open_memory().await.unwrap();
        save_match(&db, 7, "demo", "src-1", "Some Title", false).await.unwrap();
        let dir = std::env::temp_dir();
        match find_match(&db, &dir, 7, "demo", &["anything".into()], None).await.unwrap() {
            MatchState::Matched { source_id, auto, .. } => assert_eq!((source_id.as_str(), auto), ("src-1", false)),
            other => panic!("{other:?}"),
        }
        clear_match(&db, 7, "demo").await.unwrap();
        // Not installed, so a fresh lookup fails rather than inventing a match.
        assert!(find_match(&db, &dir, 7, "demo", &["anything".into()], None).await.is_err());
    }

    #[tokio::test]
    async fn positions_round_trip() {
        let db = crate::db::open_memory().await.unwrap();
        sqlx::query("INSERT INTO media (id, kind, title) VALUES (3, 'ANIME', 'x')").execute(&db).await.unwrap();
        save_position(&db, 3, 2, 125.4, 1440.0).await.unwrap();
        assert_eq!(position(&db, 3, 2).await.unwrap(), Some(125));
        assert_eq!(position(&db, 3, 9).await.unwrap(), None);
    }
}
