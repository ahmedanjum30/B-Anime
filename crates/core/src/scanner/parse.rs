//! Anime video filename parsing: release group, title, season, episode.
//!
//! Handles the common naming styles without requiring any of them:
//! `[Group] Title - 05 [1080p][ABCD1234].mkv`, `Title S02E05 1080p WEB.mkv`, `Title.EP05.mkv`,
//! `Title 2nd Season - 05v2.mkv`, `05 - Episode Title.mkv` inside a `Title` folder, and so on.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// Clean title, without season markers.
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<f64>,
    pub group: Option<String>,
    pub year: Option<u32>,
    /// Openings, endings, trailers and other extras: not episodes.
    pub extra: bool,
}

impl Parsed {
    /// The text to match against AniList titles ("Title season 2" for later seasons).
    pub fn query(&self) -> String {
        match self.season {
            Some(s) if s > 1 => format!("{} season {s}", self.title),
            _ => self.title.clone(),
        }
    }
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).expect("valid regex"));
    };
}

re!(LEADING_GROUP, r"^\s*\[([^\]]+)\]");
re!(BRACKETS, r"\[[^\]]*\]|\{[^}]*\}");
re!(PARENS, r"\(([^)]*)\)");
re!(YEAR, r"^(19[5-9]\d|20\d\d)$");
re!(SXXEYY, r"(?i)\bS(\d{1,2})\s?E(\d{1,4}(?:\.\d)?)(?:v\d)?\b");
re!(NXNN, r"(?i)\b(\d{1,2})x(\d{1,3})\b");
re!(DASH_EP, r"\s-\s(\d{1,4}(?:\.\d)?)(?:v\d)?(?:\s|$)");
re!(MARKED_EP, r"(?i)(?:\b(?:ep|episode|e)\.?\s?|#|第)(\d{1,4}(?:\.\d)?)(?:v\d)?(?:話|\b)");
re!(TRAILING_NUM, r"(?:^|\s)(\d{1,4}(?:\.\d)?)(?:v\d)?\s*$");
re!(SEASON_ORDINAL, r"(?i)\b(\d{1,2})(?:st|nd|rd|th)\s+season\b");
re!(SEASON_WORD, r"(?i)\bseason\s*(\d{1,2})\b");
re!(SEASON_SHORT, r"(?i)\bS(\d{1,2})\b");
re!(ROMAN_SEASON, r"\s(II|III|IV|V|VI)$");
re!(EXTRA, r"(?i)\b(NCOP\d*|NCED\d*|creditless|OP\d*|ED\d*|opening|ending|trailer|teaser|preview|PV\d*|CM\d*|menu|sample)\b");
re!(LEADING_EP, r"^(\d{1,4})(?:v\d)?(?:\s+-\s+|\s*$)");
re!(QUALITY, r"(?i)\b(\d{3,4}p|[248]k|x26[45]|h\.?26[45]|hevc|avc|aac\d?(\.\d)?|flac|opus|ac3|e-?ac-?3|dts|ddp?\d(\.\d)?|10-?bit|8-?bit|hdr|bd(rip)?|blu-?ray|web(-?dl|-?rip)?|hdtv|dual[- ]audio|multi[- ]subs?|eng?[- ]?subs?|uncensored|batch|complete|remux|v\d)\b");

const EXTENSIONS: &[&str] = &["mkv", "mp4", "avi", "webm", "m4v", "mov", "ts", "m2ts", "wmv", "flv", "ogm"];

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn tidy(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_matches(|c: char| c.is_whitespace() || "-–_.,:;~".contains(c)).to_owned()
}

/// Strips separators, tags and quality words, keeping what reads like the title.
fn clean(raw: &str) -> (String, Option<String>, Option<u32>) {
    let group = LEADING_GROUP.captures(raw).map(|c| c[1].trim().to_owned());
    let mut s = LEADING_GROUP.replace(raw, "").into_owned();
    s = BRACKETS.replace_all(&s, " ").into_owned();
    let mut year = None;
    s = PARENS
        .replace_all(&s, |c: &regex::Captures| {
            let inner = c[1].trim();
            if YEAR.is_match(inner) {
                year = inner.parse().ok();
            }
            " "
        })
        .into_owned();
    // Dots and underscores are separators when the name has no spaces ("Title.S01E05.1080p").
    if !s.contains(' ') || s.matches('_').count() > 1 {
        s = s.replace(['_', '.'], " ");
    }
    // ...but keep "5.5" style decimal episodes intact.
    (s, group, year)
}

fn strip_quality(s: &str) -> String {
    tidy(&QUALITY.replace_all(s, " "))
}

/// The specific title some releases put in season folders: "[Judas] Gintama S07 - Gintama°"
/// → "Gintama°". Only when the part before " - " carries a season marker, so ordinary
/// "Show - Subtitle" folders aren't misread.
pub fn folder_title(folder: &str) -> Option<String> {
    let (cleaned, _, _) = clean(folder);
    let text = QUALITY.replace_all(&cleaned, " ");
    let (before, after) = text.trim().rsplit_once(" - ")?;
    let marked = SEASON_SHORT.is_match(before) || SEASON_WORD.is_match(before) || SEASON_ORDINAL.is_match(before);
    // Only whitespace is trimmed: a trailing "_" or "." is part of titles like "Gintama.".
    let title = after.split_whitespace().collect::<Vec<_>>().join(" ");
    (marked && title.chars().any(char::is_alphabetic)).then_some(title)
}

/// Parses a file name; `folder` (the parent directory's name) fills in a missing title.
pub fn parse(file_name: &str, folder: Option<&str>) -> Parsed {
    let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or(file_name);
    let extra = EXTRA.is_match(&BRACKETS.replace_all(stem, " "));
    let (cleaned, group, year) = clean(stem);
    let text = strip_quality(&cleaned);

    let mut season = None;
    let mut episode = None;
    let mut title_end = text.len();

    // "05 - Episode Title" inside a show's folder: the number is the episode, the folder the title.
    if let (Some(_), Some(c)) = (folder, LEADING_EP.captures(&text)) {
        episode = c[1].parse().ok();
        title_end = 0;
    } else if let Some(c) = SXXEYY.captures(&text) {
        season = c[1].parse().ok();
        episode = c[2].parse().ok();
        title_end = c.get(0).unwrap().start();
    } else if let Some(c) = NXNN.captures(&text) {
        season = c[1].parse().ok();
        episode = c[2].parse().ok();
        title_end = c.get(0).unwrap().start();
    } else if let Some(c) = DASH_EP.captures(&text) {
        episode = c[1].parse().ok();
        title_end = c.get(0).unwrap().start();
    } else if let Some(c) = MARKED_EP.captures(&text) {
        episode = c[1].parse().ok();
        title_end = c.get(0).unwrap().start();
    } else if let Some(c) = TRAILING_NUM.captures(&text) {
        let n: f64 = c[1].parse().unwrap_or(-1.0);
        let looks_like_year = (1950.0..2100.0).contains(&n) && c[1].len() == 4;
        if !looks_like_year {
            episode = Some(n);
            title_end = c.get(1).unwrap().start();
        }
    }

    let mut title = tidy(&text[..title_end]);

    // Season markers in the title itself.
    for re in [&*SEASON_ORDINAL, &*SEASON_WORD, &*SEASON_SHORT] {
        if let Some(c) = re.captures(&title) {
            season = season.or_else(|| c[1].parse().ok());
            title = tidy(&re.replace(&title, " "));
        }
    }
    if let Some(c) = ROMAN_SEASON.captures(&title) {
        let n = match &c[1] {
            "II" => 2,
            "III" => 3,
            "IV" => 4,
            "V" => 5,
            _ => 6,
        };
        season = season.or(Some(n));
        title = tidy(&ROMAN_SEASON.replace(&title, ""));
    }

    if title.is_empty() || title.chars().all(|c| c.is_ascii_digit()) {
        if let Some(f) = folder {
            let from_folder = parse(&format!("{f}.mkv"), None);
            title = from_folder.title;
            season = season.or(from_folder.season);
        }
    }

    Parsed { title, season, episode, group, year, extra }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> Parsed {
        parse(name, None)
    }

    #[test]
    fn fansub_style() {
        let x = p("[SubsPlease] Sousou no Frieren - 18 (1080p) [ABCD1234].mkv");
        assert_eq!(x.title, "Sousou no Frieren");
        assert_eq!(x.episode, Some(18.0));
        assert_eq!(x.group.as_deref(), Some("SubsPlease"));
        assert!(!x.extra);
    }

    #[test]
    fn season_episode_style() {
        let x = p("Dandadan.S02E09.1080p.WEB.H264-GROUP.mkv");
        assert_eq!((x.title.as_str(), x.season, x.episode), ("Dandadan", Some(2), Some(9.0)));
        assert_eq!(x.query(), "Dandadan season 2");
    }

    #[test]
    fn ordinal_seasons_and_versions() {
        let x = p("[Group] Black Clover 2nd Season - 07v2 [1080p].mkv");
        assert_eq!((x.title.as_str(), x.season, x.episode), ("Black Clover", Some(2), Some(7.0)));
        let y = p("Kaguya-sama wa Kokurasetai Season 3 - 05.mkv");
        assert_eq!((y.title.as_str(), y.season, y.episode), ("Kaguya-sama wa Kokurasetai", Some(3), Some(5.0)));
    }

    #[test]
    fn marked_and_trailing_episodes() {
        assert_eq!(p("Mushishi EP03 (BD 1080p).mkv").episode, Some(3.0));
        assert_eq!(p("Mushishi - Episode 3.mkv").episode, Some(3.0));
        assert_eq!(p("One Piece 1080.mkv").episode, Some(1080.0));
        assert_eq!(p("One Piece 1080.mkv").title, "One Piece");
        assert_eq!(p("Spy x Family - 12.5.mkv").episode, Some(12.5));
    }

    #[test]
    fn years_are_not_episodes() {
        let x = p("Kimi no Na wa (2016) [BD 1080p].mkv");
        assert_eq!((x.title.as_str(), x.episode, x.year), ("Kimi no Na wa", None, Some(2016)));
        assert_eq!(p("Blade Runner 2049.mkv").episode, None);
    }

    #[test]
    fn folder_fills_in_the_title() {
        let x = parse("05 - The Journey's End.mkv", Some("[Group] Sousou no Frieren (2023) [BD]"));
        assert_eq!((x.title.as_str(), x.episode), ("Sousou no Frieren", Some(5.0)));
        let y = parse("Episode 05.mkv", Some("Frieren"));
        assert_eq!((y.title.as_str(), y.episode), ("Frieren", Some(5.0)));
    }

    #[test]
    fn season_folders_name_the_entry() {
        assert_eq!(folder_title("[Judas] Gintama S07 - Gintama°").as_deref(), Some("Gintama°"));
        assert_eq!(folder_title("[Judas] Gintama S08 - Gintama_").as_deref(), Some("Gintama_"));
        assert_eq!(
            folder_title("[Judas] Gintama S10 - Gintama. Shirogane no Tamashii-hen").as_deref(),
            Some("Gintama. Shirogane no Tamashii-hen")
        );
        assert_eq!(folder_title("Re Zero - Starting Life in Another World"), None);
        assert_eq!(folder_title("Frieren"), None);
        let x = p("[Judas] Gintama - S07E12 (279).mkv");
        assert_eq!((x.title.as_str(), x.season, x.episode), ("Gintama", Some(7), Some(12.0)));
    }

    #[test]
    fn extras_are_flagged() {
        assert!(p("[Group] Frieren - NCOP1 [1080p].mkv").extra);
        assert!(p("Frieren - Creditless Ending.mkv").extra);
        assert!(!p("Opening Night Club - 01.mkv").extra || p("Opening Night Club - 01.mkv").episode == Some(1.0));
    }

    #[test]
    fn video_extensions() {
        assert!(is_video(Path::new("a/b.MKV")));
        assert!(!is_video(Path::new("a/b.ass")));
    }
}
