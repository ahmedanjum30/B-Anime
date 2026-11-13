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
    /// Audio language of this source: "ja" / "en", or "sub" / "dub".
    #[serde(default)]
    audio: Option<String>,
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
    /// "hls", "mp4", or "file" (a library file).
    pub kind: String,
    pub label: String,
    /// "ja" or "en" when the source says which audio it carries.
    pub audio: Option<String>,
}

/// Normalises a source's audio label to "ja" / "en" (or passes other codes through).
fn audio_code(raw: Option<&str>) -> Option<String> {
    let r = raw?.trim().to_ascii_lowercase();
    Some(match r.as_str() {
        "sub" | "ja" | "jp" | "jpn" | "japanese" | "raw" => "ja".into(),
        "dub" | "en" | "eng" | "english" => "en".into(),
        "" => return None,
        other => other.to_owned(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSubtitle {
    /// WebVTT, or for `images` a [`crate::pgs::Track`] as JSON.
    pub url: String,
    pub lang: String,
    pub label: String,
    /// Picture subtitles (PGS from a library MKV) the player draws over the video itself.
    pub images: bool,
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

/// `dub` asks the source for its English-dubbed version (sources without one ignore it).
/// `media` describes the show when known: `{ id, titles, year }` (AniList id, titles best first).
pub async fn search(db: &Db, dir: &Path, ext: &str, query: &str, dub: bool, media: Option<Value>) -> Result<Vec<SourceResult>> {
    search_with(db, dir, ext, query, dub, media, Limits::default()).await
}

async fn search_with(
    db: &Db,
    dir: &Path,
    ext: &str,
    query: &str,
    dub: bool,
    media: Option<Value>,
    limits: Limits,
) -> Result<Vec<SourceResult>> {
    let mut q = json!({ "query": query, "dub": dub });
    if let Some(media) = media {
        if let Some(year) = media.get("year").filter(|y| !y.is_null()) {
            q["year"] = year.clone();
        }
        q["media"] = media;
    }
    let out = extensions::call(db, dir, ext, "search", &json!([q]), limits).await?.value;
    parse(out, "search results")
}

/// The whole automatic lookup (all searches together) gives up after this long.
const MATCH_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
/// Each search on its own: a bit under the deadline, so a stuck one is cut off cleanly.
const MATCH_SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// The saved match for this show on this source (sub or dub), or the best automatic one
/// (saved when confident).
pub async fn find_match(
    db: &Db,
    dir: &Path,
    media_id: i64,
    ext: &str,
    titles: &[String],
    year: Option<i64>,
    dub: bool,
) -> Result<MatchState> {
    let saved: Option<(String, String, bool)> = sqlx::query_as(
        "SELECT source_id, source_title, auto FROM source_match WHERE media_id = ? AND ext_id = ? AND dub = ?",
    )
    .bind(media_id)
    .bind(ext)
    .bind(dub)
    .fetch_optional(db)
    .await?;
    if let Some((source_id, title, auto)) = saved {
        return Ok(MatchState::Matched { source_id, title, auto });
    }

    // Every search runs at once; results are scored as they arrive. The lookup ends at the
    // first exact hit, when all searches are done, or at the deadline, using whatever came back.
    use futures_util::StreamExt;
    let media = json!({ "id": media_id, "titles": titles, "year": year });
    let (queries, compare) = match_titles(titles);
    let started = std::time::Instant::now();
    tracing::info!(media_id, ext, dub, searches = queries.len(), "matching: looking the show up on the source");
    let limits = Limits { call_timeout: MATCH_SEARCH_TIMEOUT, ..Limits::default() };
    let mut pending: futures_util::stream::FuturesUnordered<_> = queries
        .iter()
        .map(|q| search_with(db, dir, ext, q, dub, Some(media.clone()), limits))
        .collect();
    let deadline = tokio::time::Instant::now() + MATCH_DEADLINE;
    let mut scored: Vec<(f64, SourceResult)> = Vec::new();
    let mut answered = 0usize;
    let mut first_error: Option<Error> = None;
    loop {
        let next = match tokio::time::timeout_at(deadline, pending.next()).await {
            Ok(Some(r)) => r,
            Ok(None) | Err(_) => break,
        };
        match next {
            Ok(results) => {
                answered += 1;
                for r in results {
                    // "(Dub)" / "(English Dub)" suffixes on dub entries shouldn't cost them the match.
                    let candidate = strip_dub_marker(&r.title);
                    let mut score = compare.iter().map(|t| similarity(t, &candidate)).fold(0.0, f64::max);
                    if let (Some(a), Some(b)) = (year, r.year) {
                        if a == b {
                            score += 0.05;
                        }
                    }
                    if !scored.iter().any(|(_, x)| x.id == r.id) {
                        scored.push((score, r));
                    }
                }
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
        if scored.iter().any(|(s, _)| *s >= 0.99) {
            break;
        }
    }
    let unfinished = pending.len();
    drop(pending);
    let ms = started.elapsed().as_millis() as u64;
    if answered == 0 {
        // Nothing usable: every search failed, or none answered in time.
        let err = first_error.unwrap_or_else(|| {
            Error::Extension(format!("the source didn't answer within {} seconds", MATCH_DEADLINE.as_secs()))
        });
        tracing::warn!(media_id, ext, ms, unfinished, error = %err, "matching: no search answered");
        return Err(err);
    }
    tracing::info!(media_id, ext, ms, answered, unfinished, results = scored.len(), "matching: searches done");
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    match scored.first() {
        Some((score, best)) if *score >= AUTO_MATCH => {
            save_match(db, media_id, ext, dub, &best.id, &best.title, true).await?;
            Ok(MatchState::Matched { source_id: best.id.clone(), title: best.title.clone(), auto: true })
        }
        _ => Ok(MatchState::Unmatched { candidates: scored.into_iter().take(12).map(|(_, r)| r).collect() }),
    }
}

pub async fn save_match(db: &Db, media_id: i64, ext: &str, dub: bool, source_id: &str, title: &str, auto: bool) -> Result<()> {
    sqlx::query(
        "INSERT INTO source_match (media_id, ext_id, dub, source_id, source_title, auto) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(media_id, ext_id, dub) DO UPDATE SET source_id = excluded.source_id,
           source_title = excluded.source_title, auto = excluded.auto, updated_at = datetime('now')",
    )
    .bind(media_id)
    .bind(ext)
    .bind(dub)
    .bind(source_id)
    .bind(title)
    .bind(auto)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn clear_match(db: &Db, media_id: i64, ext: &str, dub: bool) -> Result<()> {
    sqlx::query("DELETE FROM source_match WHERE media_id = ? AND ext_id = ? AND dub = ?")
        .bind(media_id)
        .bind(ext)
        .bind(dub)
        .execute(db)
        .await?;
    Ok(())
}

/// Titles past this length are searched by their short form: light-novel titles run to whole
/// sentences, which sites list (and search engines match) only by their opening clause.
const LONG_TITLE: usize = 60;
/// At most this many searches per match attempt (matching stops early on an exact hit).
const MAX_QUERIES: usize = 4;

/// Symbols sites spell out in plain text: "SPY×FAMILY" → "SPY x FAMILY", typographic dashes
/// and quotes → ASCII.
pub fn plain_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for c in title.chars() {
        match c {
            '×' | '✕' | '✖' => out.push_str(" x "),
            '–' | '—' | '‐' => out.push('-'),
            '’' | '‘' | '`' => out.push('\''),
            '“' | '”' => out.push('"'),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

static SEASON_N: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bseason\s+(\d+)\b").unwrap());
static NTH_SEASON: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\b(\d+)(?:st|nd|rd|th)\s+season\b").unwrap());
static SEASON_MARKER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(?:(?:season|part|cour)\s+\d+|\d+(?:st|nd|rd|th)\s+(?:season|part|cour)|final\s+season)\b").unwrap()
});

/// The other common way of writing a season: "Season 2" ↔ "2nd Season".
pub fn swap_season(title: &str) -> Option<String> {
    if let Some(c) = SEASON_N.captures(title) {
        let n: u32 = c[1].parse().ok()?;
        return Some(SEASON_N.replace(title, format!("{} Season", ordinal(n))).into_owned());
    }
    let c = NTH_SEASON.captures(title)?;
    Some(NTH_SEASON.replace(title, format!("Season {}", &c[1])).into_owned())
}

/// The title without its season or part ("SPY x FAMILY Season 2" → "SPY x FAMILY"): a broad
/// search that returns every season, for the scoring to pick from.
pub fn base_title(title: &str) -> Option<String> {
    let stripped = SEASON_MARKER.replace_all(title, " ");
    let base = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    let base = base.trim_matches(|c: char| c == ':' || c == '-' || c == ',' || c.is_whitespace()).to_owned();
    (base.chars().count() >= 4 && base.len() < title.trim().len()).then_some(base)
}

/// The opening clause of a title, cut at the first ":", ". ", "? ", "! " or " - " that leaves
/// at least 12 characters ("Dr. STONE: Science Future" → "Dr. STONE").
pub fn short_title(title: &str) -> Option<String> {
    let t = title.trim();
    let mut cuts: Vec<usize> = [":", ". ", "? ", "! ", " - ", " – "].iter().flat_map(|sep| t.match_indices(sep).map(|(i, _)| i)).collect();
    cuts.sort_unstable();
    cuts.into_iter().find_map(|i| {
        let s = t[..i].trim_end_matches(['.', ' ', ',']).trim();
        (s.chars().count() >= 12).then(|| s.to_owned())
    })
}

/// What to search a source for (best first, at most [`MAX_QUERIES`]) and what to score its
/// results against.
///
/// Searches go from exact to loose: each title as AniList has it (long ones by their short
/// form), short forms, symbols in plain text ("SPY x FAMILY"), the other season wording
/// ("2nd Season"), and finally the title without its season, which returns every season for
/// the scoring to choose from.
fn match_titles(titles: &[String]) -> (Vec<String>, Vec<String>) {
    let titles: Vec<&str> = titles.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    let main: Vec<&str> = titles.iter().take(2).copied().collect();
    let shorts: Vec<Option<String>> = titles.iter().map(|t| short_title(t)).collect();
    let mut queries: Vec<String> = Vec::new();
    let add = |q: &str, list: &mut Vec<String>| {
        if !q.is_empty() && !list.iter().any(|x| x.eq_ignore_ascii_case(q)) {
            list.push(q.to_owned());
        }
    };
    for (t, short) in main.iter().zip(&shorts) {
        match short {
            Some(s) if t.chars().count() > LONG_TITLE => add(s, &mut queries),
            _ => add(t, &mut queries),
        }
    }
    for s in shorts.iter().take(2).flatten() {
        add(s, &mut queries);
    }
    // Searchable form of each main title (plain symbols, long ones shortened).
    let searchable: Vec<String> = main
        .iter()
        .zip(&shorts)
        .map(|(t, short)| {
            let t = match short {
                Some(s) if t.chars().count() > LONG_TITLE => s.as_str(),
                _ => t,
            };
            plain_title(t)
        })
        .collect();
    for t in &searchable {
        add(t, &mut queries);
    }
    for t in &searchable {
        if let Some(s) = swap_season(t) {
            add(&s, &mut queries);
        }
    }
    for t in &searchable {
        if let Some(b) = base_title(t) {
            add(&b, &mut queries);
        }
    }
    queries.truncate(MAX_QUERIES);

    let mut compare: Vec<String> = Vec::new();
    for t in &titles {
        add(t, &mut compare);
        let plain = plain_title(t);
        add(&plain, &mut compare);
        if let Some(s) = swap_season(&plain) {
            add(&s, &mut compare);
        }
    }
    for s in shorts.into_iter().flatten() {
        add(&s, &mut compare);
    }
    (queries, compare)
}

/// "Frieren (Dub)" → "Frieren", for scoring dub entries against AniList titles.
fn strip_dub_marker(title: &str) -> String {
    let lower = title.to_lowercase();
    for marker in ["(english dub)", "(dub)", "[dub]", " dub", "(eng dub)"] {
        if let Some(i) = lower.rfind(marker) {
            if i + marker.len() >= lower.trim_end().len() {
                return title[..i].trim_end().to_owned();
            }
        }
    }
    title.to_owned()
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
            audio: audio_code(s.audio.as_deref()),
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
            images: false,
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
    fn long_titles_search_by_their_opening_clause() {
        let romaji = "Tsuihou Sareta Cheat Fuyo Majutsushi wa Kimama na Second Life wo Ouka Suru.: Ore wa Buki dake ja Naku, Arayuru Mono ni \"Kyouka Point\" wo Fuyo Dekiru shi";
        let english = "The Laid-Off Cheat-Granting Mage Enjoys a Second Lease on Life";
        assert_eq!(short_title(romaji).as_deref(), Some("Tsuihou Sareta Cheat Fuyo Majutsushi wa Kimama na Second Life wo Ouka Suru"));
        assert_eq!(short_title("Dr. STONE: SCIENCE FUTURE Part 3"), None, "too short to stand alone");
        assert_eq!(short_title("Kusuriya no Hitorigoto: Season 2").as_deref(), Some("Kusuriya no Hitorigoto"));
        assert_eq!(short_title("Frieren"), None);

        let (queries, compare) = match_titles(&[romaji.into(), english.into()]);
        assert_eq!(queries, [short_title(romaji).unwrap(), english.to_owned()]);
        assert!(compare.contains(&short_title(romaji).unwrap()));
        // A site listing the short romaji title now clears the auto-match bar.
        let best = compare.iter().map(|t| similarity(t, "Tsuihou Sareta Cheat Fuyo Majutsushi wa Kimama na Second Life wo Ouka Suru")).fold(0.0, f64::max);
        assert!(best >= AUTO_MATCH, "{best}");

        // Ordinary titles search exactly first, then the other season wording.
        let (queries, _) = match_titles(&["Kusuriya no Hitorigoto 2nd Season".into(), "The Apothecary Diaries Season 2".into()]);
        assert_eq!(
            queries,
            [
                "Kusuriya no Hitorigoto 2nd Season",
                "The Apothecary Diaries Season 2",
                "Kusuriya no Hitorigoto Season 2",
                "The Apothecary Diaries 2nd Season"
            ]
        );
        let (queries, _) = match_titles(&["Sousou no Frieren: Season 2".into(), "Frieren: Beyond Journey's End Season 2".into()]);
        assert_eq!(queries[..3], ["Sousou no Frieren: Season 2", "Frieren: Beyond Journey's End Season 2", "Sousou no Frieren"]);
    }

    #[test]
    fn season_and_symbol_spellings_are_searched_too() {
        assert_eq!(plain_title("SPY×FAMILY Season 2"), "SPY x FAMILY Season 2");
        assert_eq!(swap_season("SPY x FAMILY Season 2").as_deref(), Some("SPY x FAMILY 2nd Season"));
        assert_eq!(swap_season("Kusuriya no Hitorigoto 3rd Season").as_deref(), Some("Kusuriya no Hitorigoto Season 3"));
        assert_eq!(swap_season("Season 11 of Something").as_deref(), Some("11th Season of Something"));
        assert_eq!(swap_season("Frieren"), None);
        assert_eq!(base_title("SPY x FAMILY Season 2").as_deref(), Some("SPY x FAMILY"));
        assert_eq!(base_title("Attack on Titan Final Season Part 2").as_deref(), Some("Attack on Titan"));
        assert_eq!(base_title("Frieren"), None);

        // AniList's titles for SPY×FAMILY Season 2; a site listing it as "SPY x FAMILY 2nd Season".
        let (queries, compare) = match_titles(&["SPY×FAMILY Season 2".into(), "SPY x FAMILY Season 2".into()]);
        assert_eq!(queries, ["SPY×FAMILY Season 2", "SPY x FAMILY Season 2", "SPY x FAMILY 2nd Season", "SPY x FAMILY"]);
        let best = compare.iter().map(|t| similarity(t, "SPY x FAMILY 2nd Season")).fold(0.0, f64::max);
        assert!(best >= 0.99, "{best}");
        // The broad search returns every season; the right one still scores highest.
        let score = |c: &str| compare.iter().map(|t| similarity(t, c)).fold(0.0, f64::max);
        assert!(score("SPY x FAMILY 2nd Season") > score("SPY x FAMILY"));
        assert!(score("SPY x FAMILY 2nd Season") > score("SPY x FAMILY Season 3"));
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
        save_match(&db, 7, "demo", false, "src-1", "Some Title", false).await.unwrap();
        save_match(&db, 7, "demo", true, "src-1-dub", "Some Title (Dub)", false).await.unwrap();
        let dir = std::env::temp_dir();
        let titles = ["anything".to_owned()];
        match find_match(&db, &dir, 7, "demo", &titles, None, false).await.unwrap() {
            MatchState::Matched { source_id, auto, .. } => assert_eq!((source_id.as_str(), auto), ("src-1", false)),
            other => panic!("{other:?}"),
        }
        // Sub and dub are matched separately.
        match find_match(&db, &dir, 7, "demo", &titles, None, true).await.unwrap() {
            MatchState::Matched { source_id, .. } => assert_eq!(source_id, "src-1-dub"),
            other => panic!("{other:?}"),
        }
        clear_match(&db, 7, "demo", false).await.unwrap();
        // Not installed, so a fresh lookup fails rather than inventing a match.
        assert!(find_match(&db, &dir, 7, "demo", &titles, None, false).await.is_err());
    }

    #[test]
    fn dub_markers_and_audio_codes() {
        assert_eq!(strip_dub_marker("Frieren (Dub)"), "Frieren");
        assert_eq!(strip_dub_marker("Frieren (English Dub)"), "Frieren");
        assert_eq!(strip_dub_marker("Dubbed Life"), "Dubbed Life");
        assert_eq!(audio_code(Some("Sub")).as_deref(), Some("ja"));
        assert_eq!(audio_code(Some("DUB")).as_deref(), Some("en"));
        assert_eq!(audio_code(Some("es")).as_deref(), Some("es"));
        assert_eq!(audio_code(None), None);
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
