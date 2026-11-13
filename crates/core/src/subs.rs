//! Subtitle conversion to WebVTT, the only format the built-in player's `<track>` reads.
//!
//! Text survives; ASS styling doesn't (fonts, colours, karaoke). What is kept: line breaks,
//! italics/bold/underline from SRT, and top placement for signs (`\an8`, top-aligned styles).
//! Vector drawings are dropped, and frame-by-frame typesetting (the same line repeated with a new
//! position every frame) is merged into one cue.

use std::collections::HashMap;

/// One subtitle line, times in milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start: u64,
    pub end: u64,
    pub text: String,
    /// Shown at the top of the picture (signs, notes) instead of the bottom.
    pub top: bool,
}

/// Renders cues as a WebVTT document. Cues may come in any order.
pub fn render(mut cues: Vec<Cue>) -> String {
    cues.retain(|c| !c.text.trim().is_empty());
    cues.sort_by_key(|c| (c.start, c.end));
    let mut merged: Vec<Cue> = Vec::with_capacity(cues.len());
    for cue in cues {
        // Same text continuing within a frame or two of an earlier cue: extend that cue instead.
        if let Some(prev) =
            merged.iter_mut().rev().take(12).find(|p| p.text == cue.text && p.top == cue.top && cue.start <= p.end + 60)
        {
            prev.end = prev.end.max(cue.end);
            continue;
        }
        merged.push(cue);
    }
    let mut out = String::with_capacity(merged.len() * 64 + 8);
    out.push_str("WEBVTT\n\n");
    for c in merged {
        let end = c.end.max(c.start + 1);
        out.push_str(&format!("{} --> {}", timestamp(c.start), timestamp(end)));
        if c.top {
            out.push_str(" line:1");
        }
        out.push('\n');
        out.push_str(&c.text);
        out.push_str("\n\n");
    }
    out
}

/// Milliseconds as a WebVTT timestamp.
pub fn timestamp(ms: u64) -> String {
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Escapes cue text and drops blank lines (a blank line would end the cue).
fn finish(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// SRT/plain cue text: keeps `<i>`, `<b>`, `<u>`, drops other tags (`<font …>`), escapes the rest.
pub fn srt_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('<') {
        out.push_str(&finish_inline(&rest[..i]));
        let after = &rest[i + 1..];
        let looks_like_tag = after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/');
        match after.find('>').filter(|j| looks_like_tag && !after[..*j].contains('<')) {
            Some(j) => {
                let tag = after[..j].trim().to_ascii_lowercase();
                if matches!(tag.as_str(), "i" | "b" | "u" | "/i" | "/b" | "/u") {
                    out.push('<');
                    out.push_str(&tag);
                    out.push('>');
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push_str("&lt;");
                rest = after;
            }
        }
    }
    out.push_str(&finish_inline(rest));
    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn finish_inline(s: &str) -> String {
    s.replace('&', "&amp;").replace('>', "&gt;")
}

/// What an ASS script's header says about placement: which styles sit at the top.
#[derive(Debug, Default, Clone)]
pub struct AssHeader {
    top_styles: HashMap<String, bool>,
    /// SSA (v4) numbers alignments differently from ASS (v4+).
    legacy: bool,
}

impl AssHeader {
    pub fn parse(script: &str) -> Self {
        let mut header = AssHeader::default();
        let mut in_styles = false;
        let mut fields: Vec<String> = Vec::new();
        for line in script.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                let section = line.to_ascii_lowercase();
                in_styles = section.contains("styles");
                if section == "[v4 styles]" {
                    header.legacy = true;
                }
                if section == "[script info]" || section == "[events]" {
                    fields.clear();
                }
                continue;
            }
            if !in_styles {
                continue;
            }
            if let Some(f) = line.strip_prefix("Format:") {
                fields = f.split(',').map(|x| x.trim().to_ascii_lowercase()).collect();
            } else if let Some(s) = line.strip_prefix("Style:") {
                let parts: Vec<&str> = s.split(',').map(str::trim).collect();
                let get = |name: &str| fields.iter().position(|f| f == name).and_then(|i| parts.get(i).copied());
                if let (Some(name), Some(align)) = (get("name"), get("alignment").and_then(|a| a.parse::<u8>().ok())) {
                    header.top_styles.insert(name.to_ascii_lowercase(), header.is_top(align, false));
                }
            }
        }
        header
    }

    fn is_top(&self, align: u8, numpad: bool) -> bool {
        if self.legacy && !numpad { matches!(align, 5..=7) } else { matches!(align, 7..=9) }
    }

    /// One event's cue text and placement; `None` for drawings and lines with no text.
    pub fn event(&self, style: &str, raw: &str) -> Option<(String, bool)> {
        let mut text = String::with_capacity(raw.len());
        let mut top = self.top_styles.get(&style.trim().to_ascii_lowercase()).copied().unwrap_or(false);
        let mut rest = raw;
        while let Some(open) = rest.find('{') {
            text.push_str(&rest[..open]);
            let Some(close) = rest[open..].find('}') else {
                rest = &rest[open + 1..];
                break;
            };
            let block = &rest[open + 1..open + close];
            for tag in block.split('\\').map(str::trim) {
                if let Some(n) = tag.strip_prefix("an").and_then(|n| n.parse::<u8>().ok()) {
                    top = self.is_top(n, true);
                } else if let Some(n) = tag.strip_prefix('a').and_then(|n| n.parse::<u8>().ok()) {
                    // Legacy \a uses the SSA numbering whatever the script version.
                    top = matches!(n, 5..=7);
                } else if let Some(n) = tag.strip_prefix('p').and_then(|n| n.parse::<u8>().ok())
                    && n > 0
                {
                    return None; // a vector drawing, not text
                }
            }
            rest = &rest[open + close + 1..];
        }
        text.push_str(rest);
        let text = text.replace("\\N", "\n").replace("\\n", "\n").replace("\\h", " ");
        let text = finish(&text);
        (!text.is_empty()).then_some((text, top))
    }
}

/// `H:MM:SS.cc` to milliseconds.
pub fn ass_time(t: &str) -> Option<u64> {
    let (hms, frac) = t.trim().split_once('.')?;
    let mut parts = hms.split(':');
    let (h, m, s): (u64, u64, u64) =
        (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    // Centiseconds normally; be lenient about other precisions.
    let digits: String = frac.chars().take(3).collect();
    let ms: u64 = format!("{digits:0<3}").parse().ok()?;
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

/// A whole ASS/SSA script as WebVTT.
pub fn ass_to_vtt(script: &str) -> String {
    let header = AssHeader::parse(script);
    let mut cues = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut in_events = false;
    for line in script.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_events = line.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(f) = line.strip_prefix("Format:") {
            fields = f.split(',').map(|x| x.trim().to_ascii_lowercase()).collect();
            continue;
        }
        let Some(d) = line.strip_prefix("Dialogue:") else { continue };
        if fields.is_empty() {
            continue;
        }
        let parts: Vec<&str> = d.splitn(fields.len(), ',').collect();
        let get = |name: &str| fields.iter().position(|f| f == name).and_then(|i| parts.get(i).copied());
        let (Some(start), Some(end), Some(raw)) = (get("start").and_then(ass_time), get("end").and_then(ass_time), get("text"))
        else {
            continue;
        };
        if let Some((text, top)) = header.event(get("style").unwrap_or(""), raw) {
            cues.push(Cue { start, end, text, top });
        }
    }
    render(cues)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "[Script Info]\nScriptType: v4.00+\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, Alignment\nStyle: Default,Arial,20,2\nStyle: Sign,Arial,20,8\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,{\\i1}Hello{\\i0}\\Nthere & you\nDialogue: 0,0:00:03.00,0:00:04.00,Sign,,0,0,0,,Shop\nDialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,{\\an8}Top note\nDialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,{\\p1}m 0 0 l 10 10{\\p0}\nDialogue: 0,0:00:07.00,0:00:07.04,Sign,,0,0,0,,{\\pos(1,1)}Moving\nDialogue: 0,0:00:07.04,0:00:07.08,Sign,,0,0,0,,{\\pos(2,1)}Moving\n";

    #[test]
    fn ass_scripts_convert() {
        let vtt = ass_to_vtt(SCRIPT);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:02.500\nHello\nthere &amp; you\n"), "{vtt}");
        assert!(vtt.contains("00:00:03.000 --> 00:00:04.000 line:1\nShop"), "style alignment 8 is top: {vtt}");
        assert!(vtt.contains("00:00:05.000 --> 00:00:06.000 line:1\nTop note"), "{vtt}");
        assert!(!vtt.contains("m 0 0"), "drawings are dropped");
        assert_eq!(vtt.matches("Moving").count(), 1, "per-frame typesetting merges: {vtt}");
        assert!(vtt.contains("00:00:07.000 --> 00:00:07.080"), "{vtt}");
    }

    #[test]
    fn srt_text_keeps_simple_tags() {
        assert_eq!(srt_text("<i>Hi</i> <font color=\"red\">a < b</font>\n\nyes"), "<i>Hi</i> a &lt; b\nyes");
    }

    #[test]
    fn times_parse() {
        assert_eq!(ass_time("0:01:02.34"), Some(62_340));
        assert_eq!(ass_time("1:00:00.5"), Some(3_600_500));
    }
}
