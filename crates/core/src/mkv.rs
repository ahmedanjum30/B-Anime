//! Subtitle tracks inside Matroska (MKV) files.
//!
//! The webview plays MKV video but ignores the subtitle tracks in it, so the player gets them
//! from here: text tracks (ASS/SSA, SRT, WebVTT) as WebVTT, Blu-ray picture tracks (PGS) as
//! positioned images (see [`crate::pgs`]). Listing tracks only reads the file's header;
//! extracting them walks the clusters once, reading block headers and seeking past video and
//! audio data. DVD picture subtitles (VobSub) are left to MPV.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use crate::subs::{self, AssHeader, Cue};

const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const CODEC_PRIVATE: u32 = 0x63A2;
const LANGUAGE: u32 = 0x22_B59C;
const LANGUAGE_BCP47: u32 = 0x22_B59D;
const NAME: u32 = 0x536E;
const FLAG_DEFAULT: u32 = 0x88;
const FLAG_FORCED: u32 = 0x55AA;
const FLAG_ENABLED: u32 = 0xB9;
const CONTENT_ENCODINGS: u32 = 0x6D80;
const CONTENT_ENCODING: u32 = 0x6240;
const CONTENT_COMPRESSION: u32 = 0x5034;
const CONTENT_ENCRYPTION: u32 = 0x5035;
const COMP_ALGO: u32 = 0x4254;
const COMP_SETTINGS: u32 = 0x4255;
const CLUSTER: u32 = 0x1F43_B675;
const CLUSTER_TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_DURATION: u32 = 0x9B;

const UNKNOWN: u64 = u64::MAX;
/// Header elements bigger than this aren't read into memory.
const MAX_HEADER: u64 = 16 * 1024 * 1024;
/// Subtitle frames bigger than this are skipped.
const MAX_FRAME: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ass,
    Srt,
    Vtt,
    /// Pictures; extracted as [`Extracted::Images`].
    Pgs,
}

/// One extracted track.
#[derive(Debug, Clone)]
pub enum Extracted {
    Vtt(String),
    /// A [`crate::pgs::Track`] as JSON, plus WebVTT with one empty cue per picture event (cue id =
    /// event index) so the player's text-track machinery handles timing and selection.
    Images { json: String, cues: String },
}

#[derive(Debug, Clone, PartialEq)]
enum Compression {
    None,
    Zlib,
    /// Header stripping: these bytes were removed from the start of every frame.
    Prefix(Vec<u8>),
    Unsupported,
}

/// A text subtitle track.
#[derive(Debug, Clone)]
pub struct SubtitleTrack {
    pub number: u64,
    pub format: Format,
    /// Two-letter code where one exists ("en", "ja"), else what the file says.
    pub language: String,
    pub name: Option<String>,
    pub default: bool,
    pub forced: bool,
    private: Vec<u8>,
    compression: Compression,
}

impl SubtitleTrack {
    /// A menu label: the track's name, or its language.
    pub fn label(&self) -> String {
        let base = self.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| language_name(&self.language).into());
        if self.forced && !base.to_ascii_lowercase().contains("forced") && !base.to_ascii_lowercase().contains("sign") {
            format!("{base} (forced)")
        } else {
            base
        }
    }
}

fn language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "ja" => "Japanese",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        "ru" => "Russian",
        "ar" => "Arabic",
        "zh" => "Chinese",
        "ko" => "Korean",
        "id" => "Indonesian",
        "pl" => "Polish",
        "tr" => "Turkish",
        "vi" => "Vietnamese",
        "th" => "Thai",
        "" | "und" => "Subtitles",
        other => other,
    }
}

/// ISO 639-2 (what Matroska stores) to the two-letter codes the app uses.
fn short_language(code: &str) -> String {
    let code = code.trim().to_ascii_lowercase();
    let base = code.split(['-', '_']).next().unwrap_or("");
    match base {
        "eng" => "en",
        "jpn" => "ja",
        "spa" => "es",
        "por" => "pt",
        "fre" | "fra" => "fr",
        "ger" | "deu" => "de",
        "ita" => "it",
        "rus" => "ru",
        "ara" => "ar",
        "chi" | "zho" => "zh",
        "kor" => "ko",
        "ind" => "id",
        "pol" => "pl",
        "tur" => "tr",
        "vie" => "vi",
        "tha" => "th",
        other => other,
    }
    .to_owned()
}

// ── Reading ──────────────────────────────────────────────────────────────────

struct Reader {
    inner: BufReader<File>,
    pos: u64,
    len: u64,
}

impl Reader {
    fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { inner: BufReader::with_capacity(64 * 1024, file), pos: 0, len })
    }

    fn byte(&mut self) -> io::Result<u8> {
        let mut b = [0u8; 1];
        self.inner.read_exact(&mut b)?;
        self.pos += 1;
        Ok(b[0])
    }

    /// An EBML variable-length integer: (value, encoded length). `raw` keeps the length marker
    /// (element IDs are written that way).
    fn vint(&mut self, raw: bool) -> io::Result<(u64, usize)> {
        let first = self.byte()?;
        let len = first.leading_zeros() as usize + 1;
        if len > 8 {
            return Err(bad("invalid length"));
        }
        let mut value = if raw { first as u64 } else { (first as u64) & (0xFF >> len) };
        for _ in 1..len {
            value = (value << 8) | self.byte()? as u64;
        }
        Ok((value, len))
    }

    /// The next element header, or `None` at the end of the file.
    fn header(&mut self) -> io::Result<Option<(u32, u64)>> {
        if self.pos >= self.len {
            return Ok(None);
        }
        let (id, id_len) = self.vint(true)?;
        if id_len > 4 {
            return Err(bad("invalid element id"));
        }
        let (size, size_len) = self.vint(false)?;
        let all_ones = (1u64 << (7 * size_len)) - 1;
        Ok(Some((id as u32, if size == all_ones { UNKNOWN } else { size })))
    }

    fn skip(&mut self, n: u64) -> io::Result<()> {
        if n == UNKNOWN || self.pos.saturating_add(n) > self.len {
            return Err(bad("element runs past the end of the file"));
        }
        self.inner.seek_relative(n as i64)?;
        self.pos += n;
        Ok(())
    }

    fn bytes(&mut self, n: u64) -> io::Result<Vec<u8>> {
        if n == UNKNOWN || self.pos.saturating_add(n) > self.len {
            return Err(bad("element runs past the end of the file"));
        }
        let mut buf = vec![0u8; n as usize];
        self.inner.read_exact(&mut buf)?;
        self.pos += n;
        Ok(buf)
    }
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("not a readable MKV file: {what}"))
}

/// Child elements of an in-memory element body.
fn children(mut buf: &[u8], mut each: impl FnMut(u32, &[u8])) {
    fn vint(buf: &[u8], raw: bool) -> Option<(u64, usize)> {
        let first = *buf.first()?;
        let len = first.leading_zeros() as usize + 1;
        if len > 8 || buf.len() < len {
            return None;
        }
        let mut v = if raw { first as u64 } else { (first as u64) & (0xFF >> len) };
        for b in &buf[1..len] {
            v = (v << 8) | *b as u64;
        }
        Some((v, len))
    }
    while let Some((id, a)) = vint(buf, true) {
        let Some((size, b)) = vint(&buf[a..], false) else { return };
        let start = a + b;
        let Some(end) = start.checked_add(size as usize).filter(|e| *e <= buf.len()) else { return };
        each(id as u32, &buf[start..end]);
        buf = &buf[end..];
    }
}

fn uint(data: &[u8]) -> u64 {
    data.iter().take(8).fold(0, |v, b| (v << 8) | *b as u64)
}

fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).trim_end_matches('\0').to_owned()
}

fn parse_track(entry: &[u8]) -> Option<SubtitleTrack> {
    let (mut number, mut kind, mut codec) = (0, 0, String::new());
    let (mut lang, mut bcp47, mut name) = (None, None, None);
    let (mut default, mut forced, mut enabled) = (true, false, true);
    let mut private = Vec::new();
    let mut compression = Compression::None;
    children(entry, |id, data| match id {
        TRACK_NUMBER => number = uint(data),
        TRACK_TYPE => kind = uint(data),
        CODEC_ID => codec = text(data),
        CODEC_PRIVATE => private = data.to_vec(),
        LANGUAGE => lang = Some(text(data)),
        LANGUAGE_BCP47 => bcp47 = Some(text(data)),
        NAME => name = Some(text(data)),
        FLAG_DEFAULT => default = uint(data) != 0,
        FLAG_FORCED => forced = uint(data) != 0,
        FLAG_ENABLED => enabled = uint(data) != 0,
        CONTENT_ENCODINGS => children(data, |id, enc| {
            if id != CONTENT_ENCODING {
                return;
            }
            children(enc, |id, body| match id {
                CONTENT_ENCRYPTION => compression = Compression::Unsupported,
                CONTENT_COMPRESSION => {
                    let (mut algo, mut settings) = (0, Vec::new());
                    children(body, |id, d| match id {
                        COMP_ALGO => algo = uint(d),
                        COMP_SETTINGS => settings = d.to_vec(),
                        _ => {}
                    });
                    compression = match algo {
                        0 => Compression::Zlib,
                        3 => Compression::Prefix(settings),
                        _ => Compression::Unsupported,
                    };
                }
                _ => {}
            });
        }),
        _ => {}
    });
    let format = match codec.as_str() {
        "S_TEXT/ASS" | "S_TEXT/SSA" | "S_ASS" | "S_SSA" => Format::Ass,
        "S_TEXT/UTF8" | "S_TEXT/ASCII" => Format::Srt,
        "S_TEXT/WEBVTT" => Format::Vtt,
        "S_HDMV/PGS" => Format::Pgs,
        _ => return None,
    };
    // Matroska's default language is English when the element is absent.
    let language = short_language(bcp47.as_deref().or(lang.as_deref()).unwrap_or("eng"));
    (kind == 17 && number > 0 && enabled && compression != Compression::Unsupported).then(|| SubtitleTrack {
        number,
        format,
        language,
        name,
        default,
        forced,
        private,
        compression,
    })
}

/// Reads the segment header: (timestamp scale in ns, text subtitle tracks). Stops at the first
/// cluster, leaving the reader there.
fn read_header(r: &mut Reader) -> io::Result<(u64, Vec<SubtitleTrack>)> {
    let mut scale = 1_000_000;
    let mut tracks = Vec::new();
    while let Some((id, size)) = r.header()? {
        match id {
            SEGMENT => continue, // descend
            CLUSTER => return Ok((scale, tracks)),
            INFO if size <= MAX_HEADER => {
                let body = r.bytes(size)?;
                children(&body, |id, d| {
                    if id == TIMESTAMP_SCALE && uint(d) > 0 {
                        scale = uint(d);
                    }
                });
            }
            TRACKS if size <= MAX_HEADER => {
                let body = r.bytes(size)?;
                children(&body, |id, d| {
                    if id == TRACK_ENTRY
                        && let Some(t) = parse_track(d)
                    {
                        tracks.push(t);
                    }
                });
            }
            _ => r.skip(size)?,
        }
    }
    Ok((scale, tracks))
}

/// The text subtitle tracks in an MKV file (reads only the header). Not an MKV: an empty list.
pub fn subtitle_tracks(path: &Path) -> io::Result<Vec<SubtitleTrack>> {
    let mut r = Reader::open(path)?;
    match r.header()? {
        Some((0x1A45_DFA3, size)) => r.skip(size)?,
        _ => return Ok(Vec::new()),
    }
    Ok(read_header(&mut r)?.1)
}

struct Frame {
    track: u64,
    start: u64,
    duration: Option<u64>,
    data: Vec<u8>,
}

/// Every subtitle track, by track number.
pub fn extract(path: &Path) -> io::Result<HashMap<u64, Extracted>> {
    let mut r = Reader::open(path)?;
    match r.header()? {
        Some((0x1A45_DFA3, size)) => r.skip(size)?,
        _ => return Ok(HashMap::new()),
    }
    let (scale, tracks) = read_header(&mut r)?;
    if tracks.is_empty() {
        return Ok(HashMap::new());
    }
    let wanted: HashMap<u64, &SubtitleTrack> = tracks.iter().map(|t| (t.number, t)).collect();
    let to_ms = |ticks: i64| (ticks.max(0) as u128 * scale as u128 / 1_000_000) as u64;
    let mut frames: Vec<Frame> = Vec::new();
    let mut cluster_time: i64 = 0;

    // read_header stopped just inside the first cluster. A damaged or partly downloaded file
    // ends the walk early; whatever was read is still used.
    let walk = (|| -> io::Result<()> {
        while let Some((id, size)) = r.header()? {
            if size == UNKNOWN && !matches!(id, SEGMENT | CLUSTER) {
                return Err(bad("unknown-size element"));
            }
            match id {
                SEGMENT | CLUSTER => {}
                CLUSTER_TIMESTAMP => cluster_time = uint(&r.bytes(size)?) as i64,
                SIMPLE_BLOCK => {
                    let end = r.pos + size;
                    let (track, _) = r.vint(false)?;
                    if wanted.contains_key(&track) && size <= MAX_FRAME {
                        if let Some(f) = read_block(&mut r, track, end, cluster_time, to_ms)? {
                            frames.push(f);
                        }
                    } else {
                        r.skip(end - r.pos)?;
                    }
                }
                BLOCK_GROUP => {
                    let end = r.pos.saturating_add(size);
                    let mut frame = None;
                    let mut duration = None;
                    while r.pos < end {
                        let Some((id, size)) = r.header()? else { break };
                        if size == UNKNOWN {
                            return Err(bad("unknown-size element"));
                        }
                        match id {
                            BLOCK => {
                                let block_end = r.pos + size;
                                let (track, _) = r.vint(false)?;
                                if !wanted.contains_key(&track) || size > MAX_FRAME {
                                    // Not a subtitle: skip the rest of the group.
                                    r.skip(end.saturating_sub(r.pos))?;
                                    break;
                                }
                                frame = read_block(&mut r, track, block_end, cluster_time, to_ms)?;
                            }
                            BLOCK_DURATION => duration = Some(to_ms(uint(&r.bytes(size)?) as i64)),
                            _ => r.skip(size)?,
                        }
                    }
                    if let Some(mut f) = frame {
                        f.duration = duration;
                        frames.push(f);
                    }
                }
                _ => r.skip(size)?,
            }
        }
        Ok(())
    })();
    if let Err(e) = walk {
        tracing::debug!(error = %e, path = %path.display(), "MKV walk ended early");
    }

    let mut out = HashMap::new();
    for track in &tracks {
        if track.format == Format::Pgs {
            let mut mine: Vec<&Frame> = frames.iter().filter(|f| f.track == track.number).collect();
            mine.sort_by_key(|f| f.start);
            let mut decoder = crate::pgs::Decoder::default();
            for f in mine {
                if let Some(data) = decode(&track.compression, &f.data) {
                    decoder.feed(f.start, &data);
                }
            }
            let pictures = decoder.finish();
            let mut cues = String::from("WEBVTT\n\n");
            for (i, e) in pictures.events.iter().enumerate() {
                cues.push_str(&format!("{i}\n{} --> {}\n\n", subs::timestamp(e.start), subs::timestamp(e.end)));
            }
            let json = serde_json::to_string(&pictures).map_err(|e| io::Error::other(e.to_string()))?;
            out.insert(track.number, Extracted::Images { json, cues });
            continue;
        }
        let header = (track.format == Format::Ass).then(|| AssHeader::parse(&String::from_utf8_lossy(&track.private)));
        let mut mine: Vec<&Frame> = frames.iter().filter(|f| f.track == track.number).collect();
        mine.sort_by_key(|f| f.start);
        let mut cues = Vec::with_capacity(mine.len());
        for (i, f) in mine.iter().enumerate() {
            let Some(data) = decode(&track.compression, &f.data) else { continue };
            let data = String::from_utf8_lossy(&data);
            // No duration stored: show it until the next line, at most five seconds.
            let end = f.start
                + f.duration.filter(|d| *d > 0).unwrap_or_else(|| {
                    mine.get(i + 1).map(|n| n.start.saturating_sub(f.start)).unwrap_or(5000).clamp(1, 5000)
                });
            let (text, top) = match &header {
                Some(h) => {
                    // ReadOrder, Layer, Style, Name, MarginL, MarginR, MarginV, Effect, Text
                    let parts: Vec<&str> = data.splitn(9, ',').collect();
                    let (Some(style), Some(raw)) = (parts.get(2), parts.get(8)) else { continue };
                    match h.event(style, raw) {
                        Some(e) => e,
                        None => continue,
                    }
                }
                None => (subs::srt_text(&data), false),
            };
            cues.push(Cue { start: f.start, end, text, top });
        }
        out.insert(track.number, Extracted::Vtt(subs::render(cues)));
    }
    Ok(out)
}

/// The rest of a block after its track number: timecode, flags, then the frame.
fn read_block(r: &mut Reader, track: u64, end: u64, cluster_time: i64, to_ms: impl Fn(i64) -> u64) -> io::Result<Option<Frame>> {
    let rest = r.bytes(end.saturating_sub(r.pos))?;
    if rest.len() < 3 {
        return Ok(None);
    }
    let relative = i16::from_be_bytes([rest[0], rest[1]]) as i64;
    let flags = rest[2];
    if flags & 0x06 != 0 {
        return Ok(None); // laced: never used for subtitles
    }
    Ok(Some(Frame { track, start: to_ms(cluster_time + relative), duration: None, data: rest[3..].to_vec() }))
}

fn decode(compression: &Compression, data: &[u8]) -> Option<Vec<u8>> {
    match compression {
        Compression::None => Some(data.to_vec()),
        Compression::Prefix(prefix) => Some([prefix.as_slice(), data].concat()),
        Compression::Zlib => {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(data).take(MAX_FRAME * 4).read_to_end(&mut out).ok()?;
            Some(out)
        }
        Compression::Unsupported => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set B_ANIME_TEST_MKV to an MKV with text subtitles to run this against a real file.
    #[test]
    #[ignore]
    fn reads_a_real_file() {
        let path = std::env::var("B_ANIME_TEST_MKV").expect("B_ANIME_TEST_MKV");
        let started = std::time::Instant::now();
        let tracks = subtitle_tracks(Path::new(&path)).unwrap();
        println!("tracks in {:?}: {tracks:#?}", started.elapsed());
        let started = std::time::Instant::now();
        let vtt = extract(Path::new(&path)).unwrap();
        println!("extracted in {:?}", started.elapsed());
        assert!(!tracks.is_empty());
        for (n, out) in &vtt {
            match out {
                Extracted::Vtt(v) => {
                    println!("track {n}: {} cues\n{}", v.matches(" --> ").count(), &v[..v.len().min(1500)]);
                    assert!(v.matches(" --> ").count() > 10);
                }
                Extracted::Images { json, cues } => {
                    println!("{}", &cues[..cues.len().min(200)]);
                    let t: serde_json::Value = serde_json::from_str(json).unwrap();
                    let events = t["events"].as_array().unwrap();
                    println!("track {n}: {}x{}, {} picture events, {} KB", t["width"], t["height"], events.len(), json.len() / 1024);
                    for e in events.iter().take(5) {
                        let imgs: Vec<_> = e["images"].as_array().unwrap().iter().map(|i| format!("{}x{} at {},{}", i["w"], i["h"], i["x"], i["y"])).collect();
                        println!("  {}..{} ms: {imgs:?}", e["start"], e["end"]);
                    }
                    assert!(events.len() > 10);
                    // Dump the first picture so it can be looked at.
                    if let Ok(dir) = std::env::var("B_ANIME_TEST_OUT") {
                        let src = events[0]["images"][0]["src"].as_str().unwrap();
                        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, src.split_once(',').unwrap().1).unwrap();
                        std::fs::write(Path::new(&dir).join(format!("pgs-{n}.png")), png).unwrap();
                    }
                }
            }
        }
    }
}
