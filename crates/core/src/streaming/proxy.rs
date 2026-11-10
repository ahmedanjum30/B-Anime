//! A loopback media proxy for the player and the manga reader.
//!
//! Sources often need headers a browser can't set (Referer, Origin) and don't send CORS headers,
//! so the webview can't load them directly. Every media URL the UI gets points here instead:
//! `http://127.0.0.1:<port>/m/<secret>/<session>?u=<target>`. The proxy adds the session's
//! headers, passes Range through (seeking), rewrites HLS playlists so segments and keys come back
//! through it, and converts SRT/ASS subtitles to WebVTT.
//!
//! Only this app can use it: it listens on loopback, every URL carries a random secret, and
//! targets get the same public-http(s)-only check as extension fetches.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::TryStreamExt;
use serde::Deserialize;

use crate::extensions::runtime::check_url;
use crate::{Error, Result};

const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;

struct Session {
    headers: Vec<(String, String)>,
    created: Instant,
}

#[derive(Clone)]
pub struct Proxy {
    port: u16,
    secret: String,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    allow_private: bool,
}

#[derive(Clone)]
struct Shared {
    secret: String,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    client: reqwest::Client,
    allow_private: bool,
}

fn token() -> String {
    format!("{:032x}", rand::random::<u128>())
}

impl Proxy {
    /// Starts the proxy on a random loopback port.
    pub async fn start() -> Result<Self> {
        Self::start_with(false).await
    }

    /// `allow_private` lets tests proxy a local server.
    pub async fn start_with(allow_private: bool) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
        let port = listener.local_addr()?.port();
        let secret = token();
        let sessions = Arc::new(Mutex::new(HashMap::new()));
        // Redirects are followed only to places the proxy may reach directly.
        let redirect = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if let Err(e) = check_url(attempt.url(), allow_private) {
                attempt.error(e)
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36")
            .connect_timeout(Duration::from_secs(15))
            .redirect(redirect)
            .build()
            .map_err(|e| Error::Network(e.to_string()))?;
        let shared = Shared { secret: secret.clone(), sessions: sessions.clone(), client, allow_private };
        let app = Router::new().route("/m/{secret}/{session}", get(handle)).with_state(shared);
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(error = %e, "media proxy stopped");
            }
        });
        tracing::info!(port, "media proxy listening");
        Ok(Self { port, secret, sessions, allow_private })
    }

    /// Registers the headers one source needs; returns a session id for [`Proxy::url`].
    pub fn session(&self, headers: &HashMap<String, String>) -> String {
        let id = token();
        let mut map = self.sessions.lock().expect("proxy sessions");
        map.retain(|_, s| s.created.elapsed() < SESSION_TTL);
        map.insert(
            id.clone(),
            Session { headers: headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(), created: Instant::now() },
        );
        id
    }

    /// The proxied form of `target`. `subtitle` asks for conversion to WebVTT ("srt", "ass").
    pub fn url(&self, session: &str, target: &str, subtitle: Option<&str>) -> Result<String> {
        let parsed = reqwest::Url::parse(target).map_err(|e| Error::Extension(format!("bad media URL {target}: {e}")))?;
        check_url(&parsed, self.allow_private).map_err(Error::Extension)?;
        let mut url = reqwest::Url::parse(&format!("http://127.0.0.1:{}/m/{}/{session}", self.port, self.secret))
            .expect("static proxy url");
        url.query_pairs_mut().append_pair("u", target);
        if let Some(kind) = subtitle {
            url.query_pairs_mut().append_pair("sub", kind);
        }
        Ok(url.to_string())
    }

    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

#[derive(Deserialize)]
struct Params {
    u: String,
    sub: Option<String>,
}

fn fail(status: StatusCode, msg: impl Into<String>) -> Response {
    let mut r = (status, msg.into()).into_response();
    r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    r
}

async fn handle(
    State(s): State<Shared>,
    Path((secret, session)): Path<(String, String)>,
    Query(p): Query<Params>,
    req_headers: HeaderMap,
) -> Response {
    if secret != s.secret {
        return fail(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(headers) = s.sessions.lock().ok().and_then(|m| m.get(&session).map(|x| x.headers.clone())) else {
        return fail(StatusCode::GONE, "this stream link expired; start playback again");
    };
    let target = match reqwest::Url::parse(&p.u) {
        Ok(u) => u,
        Err(_) => return fail(StatusCode::BAD_REQUEST, "bad target"),
    };
    if let Err(e) = check_url(&target, s.allow_private) {
        return fail(StatusCode::FORBIDDEN, e);
    }

    let mut req = s.client.get(target.clone());
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(range) = req_headers.get(header::RANGE) {
        req = req.header(header::RANGE, range.clone());
    }
    let res = match req.send().await {
        Ok(r) => r,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, format!("source unreachable: {e}")),
    };
    let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let final_url = res.url().clone();

    let is_playlist = content_type.contains("mpegurl") || final_url.path().ends_with(".m3u8") || target.path().ends_with(".m3u8");
    if is_playlist || p.sub.is_some() {
        let body = match read_capped(res, MAX_PLAYLIST_BYTES).await {
            Ok(b) => b,
            Err(e) => return fail(StatusCode::BAD_GATEWAY, e),
        };
        let text = String::from_utf8_lossy(&body).into_owned();
        let (out, ctype) = if let Some(kind) = p.sub.as_deref() {
            (to_vtt(&text, kind), "text/vtt; charset=utf-8")
        } else {
            let base = format!("/m/{}/{session}", s.secret);
            (rewrite_playlist(&text, &final_url, |abs| proxied_path(&base, abs)), "application/vnd.apple.mpegurl")
        };
        let mut r = (status, out).into_response();
        r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
        r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
        r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return r;
    }

    // Everything else (segments, mp4, images) streams straight through.
    let mut out_headers = HeaderMap::new();
    for name in [
        reqwest::header::CONTENT_TYPE,
        reqwest::header::CONTENT_LENGTH,
        reqwest::header::CONTENT_RANGE,
        reqwest::header::ACCEPT_RANGES,
    ] {
        if let Some(v) = res.headers().get(&name).and_then(|v| HeaderValue::from_bytes(v.as_bytes()).ok()) {
            out_headers.insert(header::HeaderName::from_bytes(name.as_str().as_bytes()).expect("header name"), v);
        }
    }
    out_headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    out_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));
    let stream = res.bytes_stream().map_err(std::io::Error::other);
    (status, out_headers, Body::from_stream(stream)).into_response()
}

async fn read_capped(mut res: reqwest::Response, cap: usize) -> std::result::Result<Vec<u8>, String> {
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > cap {
            return Err("response too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn proxied_path(base: &str, absolute: &str) -> String {
    let mut q = String::new();
    for b in absolute.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            q.push(b as char);
        } else {
            q.push_str(&format!("%{b:02X}"));
        }
    }
    format!("{base}?u={q}")
}

/// Points every URI in an HLS playlist (segment lines and `URI="…"` attributes) back through the
/// proxy, resolved against the playlist's own URL.
pub fn rewrite_playlist(body: &str, base: &reqwest::Url, make: impl Fn(&str) -> String) -> String {
    let resolve = |raw: &str| base.join(raw.trim()).map(|u| make(u.as_str())).unwrap_or_else(|_| raw.to_owned());
    let mut out = String::with_capacity(body.len() * 2);
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        if trimmed.starts_with('#') {
            let mut rest = trimmed;
            let mut rewritten = String::new();
            while let Some(i) = rest.find("URI=\"") {
                let start = i + 5;
                let Some(len) = rest[start..].find('"') else { break };
                rewritten.push_str(&rest[..start]);
                rewritten.push_str(&resolve(&rest[start..start + len]));
                rest = &rest[start + len..];
            }
            rewritten.push_str(rest);
            out.push_str(&rewritten);
        } else {
            out.push_str(&resolve(trimmed));
        }
        out.push('\n');
    }
    out
}

/// Converts SRT or ASS/SSA subtitles to WebVTT (text only; ASS styling is dropped).
pub fn to_vtt(text: &str, kind: &str) -> String {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with("WEBVTT") {
        return text.to_owned();
    }
    match kind {
        "ass" | "ssa" => ass_to_vtt(text),
        _ => srt_to_vtt(text),
    }
}

fn srt_to_vtt(text: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for line in text.lines() {
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn ass_time(t: &str) -> Option<String> {
    // H:MM:SS.cc -> HH:MM:SS.mmm
    let (hms, cs) = t.trim().split_once('.')?;
    let mut parts = hms.split(':');
    let (h, m, s) = (parts.next()?.parse::<u32>().ok()?, parts.next()?.parse::<u32>().ok()?, parts.next()?.parse::<u32>().ok()?);
    let cs: u32 = cs.parse().ok()?;
    Some(format!("{h:02}:{m:02}:{s:02}.{:03}", cs * 10))
}

fn ass_to_vtt(text: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    let mut fields: Vec<String> = Vec::new();
    let mut in_events = false;
    for line in text.lines() {
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
        // Drop {\override} blocks; \N and \n are line breaks, \h is a space.
        let mut cleaned = String::new();
        let mut depth = 0;
        for ch in raw.chars() {
            match ch {
                '{' => depth += 1,
                '}' if depth > 0 => depth -= 1,
                _ if depth == 0 => cleaned.push(ch),
                _ => {}
            }
        }
        let cleaned = cleaned.replace("\\N", "\n").replace("\\n", "\n").replace("\\h", " ");
        if cleaned.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("{start} --> {end}\n{}\n\n", cleaned.trim()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlists_are_rewritten() {
        let base = reqwest::Url::parse("https://cdn.example.com/show/ep1/master.m3u8").unwrap();
        let body = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:4.0,\nseg-1.ts\n#EXTINF:4.0,\nhttps://other.example.com/seg-2.ts\n";
        let out = rewrite_playlist(body, &base, |abs| format!("P[{abs}]"));
        assert!(out.contains("URI=\"P[https://cdn.example.com/show/ep1/key.bin]\""));
        assert!(out.contains("\nP[https://cdn.example.com/show/ep1/seg-1.ts]\n"));
        assert!(out.contains("\nP[https://other.example.com/seg-2.ts]\n"));
        assert!(out.starts_with("#EXTM3U\n"));
    }

    #[test]
    fn subtitles_become_vtt() {
        let srt = "1\n00:00:01,000 --> 00:00:02,500\nHello\n";
        assert_eq!(to_vtt(srt, "srt"), "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.500\nHello\n");

        let ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\i1}Hi,{\\i0} there\\Nfriend\n";
        assert_eq!(to_vtt(ass, "ass"), "WEBVTT\n\n00:00:01.500 --> 00:00:03.000\nHi, there\nfriend\n\n");
    }

    #[tokio::test]
    async fn proxies_with_headers_and_rewrites() {
        use axum::routing::get as route;
        async fn needs_referer(headers: HeaderMap) -> (StatusCode, &'static str) {
            if headers.get("referer").and_then(|v| v.to_str().ok()) == Some("https://site.example/") {
                (StatusCode::OK, "#EXTM3U\n#EXTINF:4,\nseg.ts\n")
            } else {
                (StatusCode::FORBIDDEN, "no referer")
            }
        }
        let app = Router::new().route("/v/index.m3u8", route(needs_referer)).route("/v/seg.ts", route(|| async { "SEGMENT" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let proxy = Proxy::start_with(true).await.unwrap();
        let session = proxy.session(&HashMap::from([("Referer".into(), "https://site.example/".into())]));
        let url = proxy.url(&session, &format!("{origin}/v/index.m3u8"), None).unwrap();

        let playlist = reqwest::get(&url).await.unwrap().text().await.unwrap();
        let seg_line = playlist.lines().find(|l| l.starts_with("/m/")).expect("rewritten segment");
        let seg = reqwest::get(format!("{}{seg_line}", proxy.origin())).await.unwrap();
        assert_eq!(seg.headers()["access-control-allow-origin"], "*");
        assert_eq!(seg.text().await.unwrap(), "SEGMENT");

        // Wrong secret is refused.
        let bad = url.replace(&proxy.secret, "nope");
        assert_eq!(reqwest::get(bad).await.unwrap().status(), 403);
    }
}
