//! The extension sandbox: QuickJS with nothing but `fetch` and `console`.
//!
//! Every call gets a fresh runtime, so extensions keep no state between calls and can't see each
//! other. Limits: a wall-clock deadline (also enforced inside JS through the interrupt handler, so
//! busy loops die), a memory cap, a stack cap, and caps on fetch response size and redirect count.
//! `fetch` speaks http(s) only and refuses this machine and private networks.

use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rquickjs::prelude::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Function, Promise};
use serde_json::{Value, json};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Whole call, including network waits.
    pub call_timeout: Duration,
    pub memory_bytes: usize,
    pub max_response_bytes: usize,
    pub max_fetches: usize,
    /// Tests only: lets fetch reach 127.0.0.1.
    pub allow_private_hosts: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            call_timeout: Duration::from_secs(30),
            memory_bytes: 96 * 1024 * 1024,
            max_response_bytes: 8 * 1024 * 1024,
            max_fetches: 60,
            allow_private_hosts: false,
        }
    }
}

/// What a call returned, plus everything it logged (shown in the Extensions screen's tester).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CallOutput {
    pub value: Value,
    pub logs: Vec<String>,
}

const PRELUDE: &str = r#"
(() => {
  const hostFetch = globalThis.__host_fetch;
  const hostLog = globalThis.__host_log;
  delete globalThis.__host_fetch;
  delete globalThis.__host_log;

  const show = (v) => {
    if (typeof v === "string") return v;
    try { return JSON.stringify(v); } catch { return String(v); }
  };
  const log = (level) => (...args) => hostLog(level, args.map(show).join(" "));
  globalThis.console = { log: log("info"), info: log("info"), debug: log("debug"), warn: log("warn"), error: log("error") };

  globalThis.fetch = async (url, options = {}) => {
    const body = options.body == null ? null : typeof options.body === "string" ? options.body : JSON.stringify(options.body);
    const r = JSON.parse(await hostFetch(String(url), JSON.stringify({
      method: options.method || "GET",
      headers: options.headers || {},
      body,
    })));
    if (r.error) throw new Error(r.error);
    const headers = {
      get: (name) => r.headers[String(name).toLowerCase()] ?? null,
      has: (name) => String(name).toLowerCase() in r.headers,
      entries: () => Object.entries(r.headers),
    };
    return {
      ok: r.status >= 200 && r.status < 300,
      status: r.status,
      url: r.url,
      headers,
      text: async () => r.body,
      json: async () => JSON.parse(r.body),
    };
  };

  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  globalThis.btoa = (input) => {
    const s = String(input);
    let out = "";
    for (let i = 0; i < s.length; i += 3) {
      const a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
      if (a > 255 || b > 255 || c > 255) throw new Error("btoa: character out of range");
      const n = (a << 16) | ((b || 0) << 8) | (c || 0);
      out += chars[(n >> 18) & 63] + chars[(n >> 12) & 63] + (i + 1 < s.length ? chars[(n >> 6) & 63] : "=") + (i + 2 < s.length ? chars[n & 63] : "=");
    }
    return out;
  };
  globalThis.atob = (input) => {
    const s = String(input).replace(/[\s=]+/g, "");
    let out = "", buffer = 0, bits = 0;
    for (const ch of s) {
      const v = chars.indexOf(ch);
      if (v < 0) throw new Error("atob: invalid character");
      buffer = (buffer << 6) | v;
      bits += 6;
      if (bits >= 8) { bits -= 8; out += String.fromCharCode((buffer >> bits) & 255); }
    }
    return out;
  };
})();
"#;

/// Calls `method` on a fresh `new Provider()` with JSON `args` (an array of arguments).
/// `__methods` returns the provider's method names, for validation at install time.
const CALL: &str = r#"
(async (method, args) => {
  if (typeof Provider !== "function") throw new Error("the extension doesn't define a Provider class");
  if (method === "__methods") {
    const names = new Set();
    for (let p = Provider.prototype; p && p !== Object.prototype; p = Object.getPrototypeOf(p))
      for (const n of Object.getOwnPropertyNames(p)) if (n !== "constructor") names.add(n);
    return JSON.stringify([...names]);
  }
  const provider = new Provider();
  if (typeof provider[method] !== "function") throw new Error(`the extension has no ${method}() method`);
  const result = await provider[method](...JSON.parse(args));
  return JSON.stringify(result === undefined ? null : result);
})
"#;

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("B-Anime/", env!("CARGO_PKG_VERSION"), " (extension)"))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client")
    })
}

/// Refuses anything but http(s) to public hosts (unless `allow_private`).
pub fn check_url(url: &reqwest::Url, allow_private: bool) -> std::result::Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("only http and https URLs are allowed, not {}", url.scheme()));
    }
    if allow_private {
        return Ok(());
    }
    let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']).to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return Err(format!("{host} is on this computer or local network"));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        let private = match ip {
            IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast(),
            IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
        };
        if private {
            return Err(format!("{host} is on this computer or local network"));
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct FetchOptions {
    #[serde(default = "get")]
    method: String,
    #[serde(default)]
    headers: serde_json::Map<String, Value>,
    body: Option<String>,
}

fn get() -> String {
    "GET".into()
}

/// The host side of `fetch`: follows up to 5 redirects (each one checked) and caps the body.
async fn host_fetch(url: String, options: String, limits: Limits) -> Value {
    match host_fetch_inner(url, options, limits).await {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    }
}

async fn host_fetch_inner(url: String, options: String, limits: Limits) -> std::result::Result<Value, String> {
    let opts: FetchOptions = serde_json::from_str(&options).map_err(|e| format!("bad fetch options: {e}"))?;
    let method = reqwest::Method::from_bytes(opts.method.to_ascii_uppercase().as_bytes())
        .map_err(|_| format!("unknown method {}", opts.method))?;
    let mut url = reqwest::Url::parse(&url).map_err(|e| format!("bad URL {url}: {e}"))?;

    for _ in 0..=5 {
        check_url(&url, limits.allow_private_hosts)?;
        let mut req = http().request(method.clone(), url.clone());
        for (k, v) in &opts.headers {
            if let Some(v) = v.as_str() {
                req = req.header(k.as_str(), v);
            }
        }
        if let Some(body) = &opts.body {
            req = req.body(body.clone());
        }
        let mut res = req.send().await.map_err(|e| format!("request to {url} failed: {e}"))?;

        if res.status().is_redirection() {
            if let Some(loc) = res.headers().get(reqwest::header::LOCATION).and_then(|l| l.to_str().ok()) {
                url = url.join(loc).map_err(|e| format!("bad redirect: {e}"))?;
                continue;
            }
        }

        let status = res.status().as_u16();
        let final_url = res.url().to_string();
        let headers: serde_json::Map<String, Value> = res
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_ascii_lowercase(), Value::from(v))))
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|e| format!("reading {url} failed: {e}"))? {
            if body.len() + chunk.len() > limits.max_response_bytes {
                return Err(format!("response from {url} is larger than {} MB", limits.max_response_bytes / 1_048_576));
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(json!({
            "status": status,
            "url": final_url,
            "headers": headers,
            "body": String::from_utf8_lossy(&body),
        }));
    }
    Err("too many redirects".into())
}

/// Runs one provider method in a fresh sandbox.
pub async fn call(code: &str, method: &str, args: &Value, limits: Limits) -> Result<CallOutput> {
    let ext_err = |m: String| Error::Extension(m);

    let rt = AsyncRuntime::new().map_err(|e| ext_err(e.to_string()))?;
    rt.set_memory_limit(limits.memory_bytes).await;
    rt.set_max_stack_size(1024 * 1024).await;
    let deadline = Instant::now() + limits.call_timeout;
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline))).await;
    let ctx = AsyncContext::full(&rt).await.map_err(|e| ext_err(e.to_string()))?;

    let logs = Arc::new(Mutex::new(Vec::<String>::new()));
    let fetches = Arc::new(Mutex::new(0usize));
    let code = code.to_owned();
    let method = method.to_owned();
    let args = serde_json::to_string(args).map_err(|e| ext_err(e.to_string()))?;
    let log_sink = logs.clone();

    let work = ctx.async_with(async move |ctx| -> std::result::Result<String, String> {
        let globals = ctx.globals();
        let sink = log_sink.clone();
        globals
            .set(
                "__host_log",
                Func::from(move |level: String, message: String| {
                    let line = format!("[{level}] {message}");
                    tracing::debug!(target: "extension", "{line}");
                    if let Ok(mut l) = sink.lock() {
                        if l.len() < 500 {
                            l.push(line);
                        }
                    }
                }),
            )
            .map_err(|e| e.to_string())?;
        let counter = fetches.clone();
        globals
            .set(
                "__host_fetch",
                Func::from(Async(move |url: String, options: String| {
                    let over: bool = counter
                        .lock()
                        .map(|mut n| {
                            *n += 1;
                            *n > limits.max_fetches
                        })
                        .unwrap_or(true);
                    async move {
                        if over {
                            return Ok::<String, rquickjs::Error>(
                                json!({ "error": format!("more than {} requests in one call", limits.max_fetches) }).to_string(),
                            );
                        }
                        Ok(host_fetch(url, options, limits).await.to_string())
                    }
                })),
            )
            .map_err(|e| e.to_string())?;

        ctx.eval::<(), _>(PRELUDE).catch(&ctx).map_err(|e| format!("sandbox setup failed: {e}"))?;
        ctx.eval::<(), _>(code).catch(&ctx).map_err(|e| format!("the extension failed to load: {e}"))?;
        let caller: Function = ctx.eval(CALL).catch(&ctx).map_err(|e| e.to_string())?;
        let promise: Promise = caller.call((method, args)).catch(&ctx).map_err(|e| e.to_string())?;
        promise.into_future::<String>().await.catch(&ctx).map_err(|e| e.to_string())
    });

    let outcome = tokio::time::timeout(limits.call_timeout + Duration::from_secs(1), work).await;
    let logs = logs.lock().map(|l| l.clone()).unwrap_or_default();
    match outcome {
        Err(_) => Err(ext_err(format!("timed out after {} s", limits.call_timeout.as_secs()))),
        Ok(Err(e)) if Instant::now() > deadline => Err(ext_err(format!("timed out after {} s ({e})", limits.call_timeout.as_secs()))),
        Ok(Err(e)) => Err(ext_err(e)),
        Ok(Ok(json)) => Ok(CallOutput {
            value: serde_json::from_str(&json).map_err(|e| ext_err(format!("returned something that isn't JSON: {e}")))?,
            logs,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick(allow_private: bool) -> Limits {
        Limits { call_timeout: Duration::from_secs(3), allow_private_hosts: allow_private, ..Limits::default() }
    }

    #[tokio::test]
    async fn runs_a_provider_method() {
        let code = r#"
            class Provider {
              async search(q) { console.log("searching", q); return [{ id: "1", title: q.query.toUpperCase() }]; }
            }"#;
        let out = call(code, "search", &json!([{ "query": "frieren" }]), quick(false)).await.unwrap();
        assert_eq!(out.value, json!([{ "id": "1", "title": "FRIEREN" }]));
        assert_eq!(out.logs, vec![r#"[info] searching {"query":"frieren"}"#]);
    }

    #[tokio::test]
    async fn lists_methods_and_reports_missing_ones() {
        let code = "class Base { episodes() {} } class Provider extends Base { search() {} }";
        let out = call(code, "__methods", &json!([]), quick(false)).await.unwrap();
        let mut names: Vec<String> = serde_json::from_value(out.value).unwrap();
        names.sort();
        assert_eq!(names, ["episodes", "search"]);
        let err = call(code, "sources", &json!([]), quick(false)).await.unwrap_err();
        assert!(err.to_string().contains("no sources() method"), "{err}");
    }

    #[tokio::test]
    async fn busy_loops_are_interrupted() {
        let limits = Limits { call_timeout: Duration::from_millis(500), ..quick(false) };
        let started = Instant::now();
        let err = call("class Provider { search() { for (;;) {} } }", "search", &json!([]), limits).await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(err.to_string().contains("timed out"), "{err}");
    }

    #[tokio::test]
    async fn memory_is_capped() {
        let limits = Limits { memory_bytes: 16 * 1024 * 1024, ..quick(false) };
        let code = "class Provider { search() { const a = []; for (;;) a.push(new Array(1e5).fill(1)); } }";
        assert!(call(code, "search", &json!([]), limits).await.is_err());
    }

    #[tokio::test]
    async fn fetch_refuses_this_machine() {
        let code = r#"class Provider { async search() { await fetch("http://127.0.0.1:9/"); } }"#;
        let err = call(code, "search", &json!([]), quick(false)).await.unwrap_err();
        assert!(err.to_string().contains("on this computer"), "{err}");
        assert!(check_url(&"file:///C:/Windows".parse().unwrap(), false).is_err());
        assert!(check_url(&"http://192.168.1.1/".parse().unwrap(), false).is_err());
        assert!(check_url(&"https://anilist.co/".parse().unwrap(), false).is_ok());
    }

    #[tokio::test]
    async fn fetch_reads_responses() {
        use axum::{Router, routing::get};
        let app = Router::new()
            .route("/data", get(|| async { ([("x-test", "yes")], r#"{"items":[1,2,3]}"#) }))
            .route("/moved", get(|| async { axum::response::Redirect::temporary("/data") }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let code = format!(
            r#"class Provider {{
                async search() {{
                  const r = await fetch("http://{addr}/moved");
                  const data = await r.json();
                  return {{ status: r.status, header: r.headers.get("X-Test"), sum: data.items.reduce((a, b) => a + b, 0), b64: btoa("hi"), back: atob("aGk=") }};
                }}
              }}"#
        );
        let out = call(&code, "search", &json!([]), quick(true)).await.unwrap();
        assert_eq!(out.value, json!({ "status": 200, "header": "yes", "sum": 6, "b64": "aGk=", "back": "hi" }));
    }
}
