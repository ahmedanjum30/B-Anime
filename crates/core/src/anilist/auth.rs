//! AniList sign-in (OAuth implicit grant) and token storage.
//!
//! Flow: we listen on a fixed loopback port, open the browser at AniList's authorize page, and
//! AniList redirects to `http://127.0.0.1:<port>/callback#access_token=...`. The fragment never
//! reaches a server, so the callback page posts it back to us from the browser. The token (valid
//! for a year) goes to the OS credential store (Windows Credential Manager, macOS Keychain,
//! Secret Service on Linux), never to disk in plain text.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use tokio::sync::{Mutex, oneshot};

use crate::{Error, Result};

/// Must match the redirect URL registered for the AniList API client. Not 43211: another
/// anime app uses that port by default, and people switching from it often still run it.
pub const CALLBACK_PORT: u16 = 48731;
pub const REDIRECT_URI: &str = "http://127.0.0.1:48731/callback";

const KEYRING_SERVICE: &str = "B-Anime";
const KEYRING_USER: &str = "anilist-token";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// B-Anime's AniList API client (anilist.co/settings/developer), redirect URL [`REDIRECT_URI`].
/// Public by design: the implicit grant uses no client secret, and none is ever shipped.
pub const DEFAULT_CLIENT_ID: &str = "56545";

/// The AniList API client id. `BANIME_ANILIST_CLIENT_ID` (run time, then build time) overrides
/// the built-in one, e.g. for a fork with its own AniList client.
pub fn client_id() -> Option<String> {
    std::env::var("BANIME_ANILIST_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("BANIME_ANILIST_CLIENT_ID").map(str::to_owned))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| Some(DEFAULT_CLIENT_ID.to_owned()))
}

pub fn authorize_url(client_id: &str) -> String {
    format!("https://anilist.co/api/v2/oauth/authorize?client_id={client_id}&response_type=token")
}

pub fn load_token() -> Result<Option<String>> {
    match entry()?.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Credentials(e.to_string())),
    }
}

pub fn save_token(token: &str) -> Result<()> {
    entry()?.set_password(token).map_err(|e| Error::Credentials(e.to_string()))
}

pub fn delete_token() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Credentials(e.to_string())),
    }
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| Error::Credentials(e.to_string()))
}

/// A sign-in in progress: the loopback server is up; open `url` in the browser, then `wait`.
pub struct PendingLogin {
    pub url: String,
    rx: oneshot::Receiver<std::result::Result<String, String>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl PendingLogin {
    /// Resolves with the access token, or fails on denial, timeout or cancellation.
    pub async fn wait(mut self) -> Result<String> {
        let outcome = tokio::time::timeout(LOGIN_TIMEOUT, &mut self.rx).await;
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
        match outcome {
            Err(_) => Err(Error::Login("sign-in timed out after 5 minutes".into())),
            Ok(Err(_)) => Err(Error::Login("sign-in was cancelled".into())),
            Ok(Ok(Err(reason))) => Err(Error::Login(reason)),
            Ok(Ok(Ok(token))) => Ok(token),
        }
    }
}

impl Drop for PendingLogin {
    fn drop(&mut self) {
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
    }
}

type Slot = Arc<Mutex<Option<oneshot::Sender<std::result::Result<String, String>>>>>;

/// Starts the loopback callback server. Fails if the port is taken (e.g. a second sign-in).
pub async fn start_login(client_id: &str) -> Result<PendingLogin> {
    let addr = SocketAddr::from(([127, 0, 0, 1], CALLBACK_PORT));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| Error::Login(format!("couldn't listen on port {CALLBACK_PORT}: {e}")))?;

    let (tx, rx) = oneshot::channel();
    let slot: Slot = Arc::new(Mutex::new(Some(tx)));
    let (stop_tx, stop_rx) = oneshot::channel::<()>();

    let app = Router::new()
        .route("/callback", get(callback_page))
        .route("/token", get(receive_token))
        .with_state(slot);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await;
    });

    Ok(PendingLogin {
        url: authorize_url(client_id),
        rx,
        shutdown: Some(stop_tx),
    })
}

/// AniList puts the token in the URL fragment; this page hands it to `/token`.
async fn callback_page() -> Html<&'static str> {
    Html(CALLBACK_HTML)
}

#[derive(Deserialize)]
struct TokenParams {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn receive_token(State(slot): State<Slot>, Query(p): Query<TokenParams>) -> &'static str {
    let result = match (p.access_token, p.error) {
        (Some(t), _) if !t.is_empty() => Ok(t),
        (_, Some(e)) => Err(p.error_description.unwrap_or(e)),
        _ => Err("AniList didn't return a token".to_owned()),
    };
    let ok = result.is_ok();
    if let Some(tx) = slot.lock().await.take() {
        let _ = tx.send(result);
    }
    if ok { "ok" } else { "error" }
}

const CALLBACK_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>B-Anime · AniList sign-in</title>
<style>
  body{margin:0;min-height:100vh;display:grid;place-items:center;background:#191b22;color:#ece7dc;
       font:15px/22px system-ui,sans-serif}
  main{max-width:420px;padding:32px}
  h1{font-size:18px;margin:0 0 8px}
  p{margin:0;color:#b9bcc6}
  .err h1{color:#e5483b}
</style></head>
<body><main id="m"><h1>Finishing sign-in…</h1><p>One moment.</p></main>
<script>
  const h = new URLSearchParams(location.hash.slice(1));
  const q = new URLSearchParams(location.search);
  const params = new URLSearchParams();
  for (const k of ["access_token", "error", "error_description"]) {
    const v = h.get(k) || q.get(k);
    if (v) params.set(k, v);
  }
  history.replaceState(null, "", "/callback");
  const m = document.getElementById("m");
  fetch("/token?" + params).then(r => r.text()).then(t => {
    if (t === "ok") {
      m.innerHTML = "<h1>Signed in to AniList</h1><p>You can close this tab and go back to B-Anime.</p>";
    } else {
      m.className = "err";
      m.innerHTML = "<h1>Sign-in didn't finish</h1><p>Go back to B-Anime and try again.</p>";
    }
  }).catch(() => {
    m.className = "err";
    m.innerHTML = "<h1>B-Anime isn't listening</h1><p>Start the sign-in again from B-Anime's Settings.</p>";
  });
</script></body></html>"#;
