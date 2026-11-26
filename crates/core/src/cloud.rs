//! Your place in each show, kept in step across your devices through your own Google account.
//!
//! Only the latest episode of each show travels (its position, length and when): one small file
//! (`progress.json`) in Google Drive's app folder (`appDataFolder`). That folder is private to
//! B-Anime and to you: it doesn't appear among your Drive files, and B-Anime's access
//! (`drive.appdata`) can't see anything else in your Drive. There's no B-Anime server.
//!
//! Sign-in happens in your browser (B-Anime never sees your password): OAuth 2.0 with PKCE and a
//! `state` check, Google answering on a one-off local address on this device. The refresh token
//! (the long-lived one) is kept like the AniList token: the OS credential store, or an owner-only
//! file in B-Anime's private folder where that store isn't used (Android, the Mac). Access tokens
//! live in memory only. Neither is ever logged or written anywhere else.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Html;
use axum::routing::get;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, oneshot};

use crate::{Error, Result};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files";
/// The app's own hidden Drive folder, and who's signed in (to show it). Nothing more.
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata openid email";
const FILE_NAME: &str = "progress.json";
const KEYRING_SERVICE: &str = "B-Anime";
const KEYRING_USER: &str = "google-refresh-token";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// B-Anime's Google OAuth client (a "Desktop app" client): from the build (`BANIME_GOOGLE_CLIENT_ID`
/// and `BANIME_GOOGLE_CLIENT_SECRET`, set from repository secrets), or the environment at run time.
/// Google treats an installed app's secret as not confidential (PKCE protects the sign-in); it's
/// still kept out of the repository. None configured: the feature stays hidden.
pub fn client() -> Option<(String, Option<String>)> {
    let get = |run: &str, build: Option<&'static str>| {
        std::env::var(run).ok().or_else(|| build.map(str::to_owned)).filter(|s| !s.trim().is_empty())
    };
    let id = get("BANIME_GOOGLE_CLIENT_ID", option_env!("BANIME_GOOGLE_CLIENT_ID"))?;
    let secret = get("BANIME_GOOGLE_CLIENT_SECRET", option_env!("BANIME_GOOGLE_CLIENT_SECRET"));
    Some((id, secret))
}

// ── The refresh token's keeping (as the AniList token's) ─────────────────────────────────────

static TOKEN_FILE: OnceLock<PathBuf> = OnceLock::new();

/// Keep the refresh token in `path` (in the app's private storage) instead of the OS credential
/// store. Call once at startup, where that store isn't used.
pub fn use_token_file(path: PathBuf) {
    let _ = TOKEN_FILE.set(path);
}

fn credentials(e: impl std::fmt::Display) -> Error {
    Error::Credentials(e.to_string())
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(credentials)
}

fn load_refresh() -> Result<Option<String>> {
    if let Some(file) = TOKEN_FILE.get() {
        return match std::fs::read_to_string(file) {
            Ok(t) if !t.trim().is_empty() => Ok(Some(t.trim().to_owned())),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(credentials(e)),
        };
    }
    match entry()?.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(credentials(e)),
    }
}

fn save_refresh(token: &str) -> Result<()> {
    if let Some(file) = TOKEN_FILE.get() {
        // Written beside, then renamed over (never half a token); owner-only on Unix.
        let part = file.with_extension("part");
        std::fs::write(&part, token).map_err(credentials)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o600)).map_err(credentials)?;
        }
        return std::fs::rename(&part, file).map_err(credentials);
    }
    entry()?.set_password(token).map_err(credentials)
}

fn delete_refresh() -> Result<()> {
    if let Some(file) = TOKEN_FILE.get() {
        return match std::fs::remove_file(file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(credentials(e)),
        };
    }
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(credentials(e)),
    }
}

/// Whether this device is signed in (a refresh token is kept).
pub fn signed_in() -> bool {
    load_refresh().ok().flatten().is_some()
}

// ── Sign-in ───────────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    /// What was actually allowed, space-separated: Google's consent screen lets you untick each
    /// permission, so it can be less than what was asked.
    scope: Option<String>,
}

/// Said when the Drive permission wasn't allowed (unticked at sign-in): without its folder
/// B-Anime has nowhere to keep your place. The sign-in is undone; it can be tried again.
pub const NO_DRIVE_ACCESS: &str = "The Drive box wasn't ticked. B-Anime needs its own folder in your Google Drive \
to keep your place (it can't see your files). Sign in again and tick \"See, create, and delete its own \
configuration data in your Google Drive\".";

/// Whether an allowed-scopes list includes B-Anime's Drive folder.
fn allows_drive(scope: &str) -> bool {
    scope.split_whitespace().any(|s| s == DRIVE_SCOPE)
}

/// A sign-in in progress: open `url` in the browser, then `wait`.
pub struct PendingSignIn {
    pub url: String,
    redirect: String,
    verifier: String,
    rx: oneshot::Receiver<std::result::Result<String, String>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for PendingSignIn {
    fn drop(&mut self) {
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
    }
}

type Slot = Arc<Mutex<Option<(String, oneshot::Sender<std::result::Result<String, String>>)>>>;

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    /// What was allowed (Google sends it back with the code).
    scope: Option<String>,
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>B-Anime</title>\
<body style=\"font:16px system-ui;background:#191b22;color:#eee;display:grid;place-items:center;height:90vh\">\
<p>You're signed in. You can close this tab and go back to B-Anime.</p>";
const FAILED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>B-Anime</title>\
<body style=\"font:16px system-ui;background:#191b22;color:#eee;display:grid;place-items:center;height:90vh\">\
<p>The sign-in didn't complete. Go back to B-Anime and try again.</p>";
const NO_DRIVE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>B-Anime</title>\
<body style=\"font:16px system-ui;background:#191b22;color:#eee;display:grid;place-items:center;height:90vh\">\
<div style=\"max-width:520px;padding:0 16px\"><p><b>The Drive box wasn't ticked.</b></p>\
<p>B-Anime needs its own folder in your Google Drive to keep your place (it can't see your files). \
Go back to B-Anime, choose Sign in with Google again, and tick \
&ldquo;See, create, and delete its own configuration data in your Google Drive&rdquo;.</p></div>";

async fn callback(State(slot): State<Slot>, Query(q): Query<Callback>) -> Html<&'static str> {
    let Some((state, tx)) = slot.lock().await.take() else { return Html(FAILED_PAGE) };
    let no_drive = q.scope.as_deref().is_some_and(|s| !allows_drive(s));
    let outcome = match (q.code, q.state, q.error) {
        (_, _, Some(e)) => Err(if e == "access_denied" { "You didn't allow access.".to_owned() } else { e }),
        // The state must be the one this sign-in made: anything else isn't ours.
        (Some(code), Some(s), None) if s == state => Ok(code),
        _ => Err("The sign-in answer didn't match this sign-in.".to_owned()),
    };
    // Without the Drive box the app says so too (after trading the code, so Google can be told
    // to forget it): the tab says what to do now.
    let page = if outcome.is_ok() && no_drive {
        NO_DRIVE_PAGE
    } else if outcome.is_ok() {
        DONE_PAGE
    } else {
        FAILED_PAGE
    };
    let _ = tx.send(outcome);
    Html(page)
}

/// Starts a sign-in: a one-off local address for Google's answer, and the browser page to open.
pub async fn start_sign_in() -> Result<PendingSignIn> {
    let (client_id, _) = client().ok_or_else(|| Error::Config("This build has no Google sign-in configured.".into()))?;
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .map_err(|e| Error::Login(format!("couldn't open the sign-in's local address: {e}")))?;
    let port = listener.local_addr().map_err(|e| Error::Login(e.to_string()))?.port();
    let redirect = format!("http://127.0.0.1:{port}/");
    // PKCE: the code Google returns is useless without this verifier, which never leaves here.
    let verifier = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>());

    let (tx, rx) = oneshot::channel();
    let slot: Slot = Arc::new(Mutex::new(Some((state.clone(), tx))));
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let app = Router::new().route("/", get(callback)).with_state(slot);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await;
    });

    let mut url = reqwest::Url::parse(AUTH_URL).map_err(|e| Error::Config(e.to_string()))?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    Ok(PendingSignIn { url: url.to_string(), redirect, verifier, rx, shutdown: Some(stop_tx) })
}

/// `key=value&…`, encoded (a form body, or a URL's query).
fn encoded(pairs: &[(&str, &str)]) -> String {
    let mut u = reqwest::Url::parse("x:/").expect("a valid URL");
    u.query_pairs_mut().extend_pairs(pairs);
    u.query().unwrap_or_default().to_owned()
}

/// A form post (`application/x-www-form-urlencoded`).
fn post_form(http: &reqwest::Client, url: &str, pairs: &[(&str, &str)]) -> reqwest::RequestBuilder {
    http.post(url).header("content-type", "application/x-www-form-urlencoded").body(encoded(pairs))
}

fn http() -> Result<reqwest::Client> {
    crate::http::client_builder()
        .user_agent(concat!("B-Anime/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Network(e.to_string()))
}

/// The email in Google's ID token (only shown to say who's signed in; not verified further, as
/// it came straight from Google's token endpoint over TLS).
fn email_of(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("email")?.as_str().map(str::to_owned)
}

impl PendingSignIn {
    /// Waits for Google's answer, then trades it for tokens: the refresh token is kept, and the
    /// signed-in email returned (to show it).
    pub async fn wait(mut self, session: &Session) -> Result<Option<String>> {
        let outcome = tokio::time::timeout(LOGIN_TIMEOUT, &mut self.rx).await;
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
        let code = match outcome {
            Err(_) => return Err(Error::Login("sign-in timed out after 5 minutes".into())),
            Ok(Err(_)) => return Err(Error::Login("sign-in was cancelled".into())),
            Ok(Ok(Err(reason))) => return Err(Error::Login(reason)),
            Ok(Ok(Ok(code))) => code,
        };
        let (client_id, secret) = client().ok_or_else(|| Error::Config("no Google client".into()))?;
        let mut form = vec![
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", self.verifier.clone()),
            ("grant_type", "authorization_code".to_owned()),
            ("redirect_uri", self.redirect.clone()),
        ];
        if let Some(s) = secret {
            form.push(("client_secret", s));
        }
        // The answer arrives while the browser is in front: on a phone, B-Anime is in the
        // background then and Android may block its network ("error sending request"). Tried
        // again for up to 2 minutes: coming back to B-Anime lets it through.
        let began = Instant::now();
        let answer: TokenAnswer = loop {
            match token_call(&form).await {
                Err(Error::Network(e)) if began.elapsed() < Duration::from_secs(120) => {
                    tracing::debug!(error = %e, "google sign-in: token request failed; trying again");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                other => break other.map_err(|e| match e {
                    Error::Network(m) => google(m),
                    e => e,
                })?,
            }
        };
        // The Drive permission unticked on Google's page: nothing to sync with. Google forgets
        // this sign-in (nothing is kept here), and the reason is said.
        if answer.scope.as_deref().is_some_and(|s| !allows_drive(s)) {
            let token = answer.refresh_token.as_deref().unwrap_or(&answer.access_token);
            if let Ok(http) = http() {
                let _ = post_form(&http, REVOKE_URL, &[("token", token)]).send().await;
            }
            return Err(Error::Login(NO_DRIVE_ACCESS.into()));
        }
        let refresh = answer
            .refresh_token
            .ok_or_else(|| Error::Login("Google didn't give a lasting sign-in; try again.".into()))?;
        save_refresh(&refresh)?;
        session.set_access(answer.access_token, answer.expires_in).await;
        Ok(answer.id_token.as_deref().and_then(email_of))
    }
}

/// A Drive answer, its failures sorted: no permission for B-Anime's folder (unticked when signing
/// in) is said as such; anything else is a network error.
async fn drive_answer(sent: reqwest::Result<reqwest::Response>) -> Result<reqwest::Response> {
    let res = sent.map_err(|e| Error::Network(e.to_string()))?;
    if res.status().as_u16() == 403 {
        let body = res.text().await.unwrap_or_default();
        // "Request had insufficient authentication scopes" (ACCESS_TOKEN_SCOPE_INSUFFICIENT).
        if body.to_ascii_lowercase().contains("insufficient") {
            return Err(Error::Login(NO_DRIVE_ACCESS.into()));
        }
        return Err(Error::Network("Google Drive answered HTTP 403".into()));
    }
    res.error_for_status().map_err(|e| Error::Network(e.to_string()))
}

/// A network failure talking to Google, said as such (the generic network error names AniList).
fn google(e: impl std::fmt::Display) -> Error {
    Error::Login(format!("couldn't reach Google: {e}"))
}

async fn token_call(form: &[(&str, String)]) -> Result<TokenAnswer> {
    let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    // A network error stays Error::Network here, so the sign-in can tell it apart and try again.
    let res = post_form(&http()?, TOKEN_URL, &pairs).send().await.map_err(|e| Error::Network(e.to_string()))?;
    let status = res.status();
    if status.as_u16() == 400 || status.as_u16() == 401 {
        // A refresh token that's been revoked (signed out elsewhere, access removed in Google's
        // settings) or expired: forget it here too.
        return Err(Error::Unauthorized);
    }
    if !status.is_success() {
        return Err(Error::Network(format!("Google answered HTTP {}", status.as_u16())));
    }
    res.json().await.map_err(|e| Error::Network(e.to_string()))
}

/// Signs this device out: Google forgets the sign-in (revoked), and this device forgets it too.
pub async fn sign_out(session: &Session) -> Result<()> {
    if let Some(token) = load_refresh()? {
        // Best effort: offline, the token is still forgotten here.
        let _ = post_form(&http()?, REVOKE_URL, &[("token", &token)]).send().await;
    }
    session.clear().await;
    delete_refresh()
}

// ── Access tokens and the file ────────────────────────────────────────────────────────────────

/// What a signed-in device holds in memory: the current access token and the file's id.
#[derive(Default)]
pub struct Session {
    access: Mutex<Option<(String, Instant)>>,
    file_id: Mutex<Option<String>>,
}

impl Session {
    async fn set_access(&self, token: String, expires_in: Option<u64>) {
        // A minute early, so it never expires in the middle of a call.
        let until = Instant::now() + Duration::from_secs(expires_in.unwrap_or(3600).saturating_sub(60));
        *self.access.lock().await = Some((token, until));
    }

    async fn clear(&self) {
        *self.access.lock().await = None;
        *self.file_id.lock().await = None;
    }

    /// A valid access token: the current one, or a new one from the kept refresh token.
    async fn token(&self) -> Result<String> {
        if let Some((t, until)) = self.access.lock().await.clone() {
            if Instant::now() < until {
                return Ok(t);
            }
        }
        let refresh = load_refresh()?.ok_or(Error::Unauthorized)?;
        let (client_id, secret) = client().ok_or_else(|| Error::Config("no Google client".into()))?;
        let mut form = vec![
            ("client_id", client_id),
            ("grant_type", "refresh_token".to_owned()),
            ("refresh_token", refresh),
        ];
        if let Some(s) = secret {
            form.push(("client_secret", s));
        }
        match token_call(&form).await {
            Ok(answer) => {
                let t = answer.access_token.clone();
                self.set_access(answer.access_token, answer.expires_in).await;
                Ok(t)
            }
            Err(Error::Unauthorized) => {
                // Revoked or expired: signed out here too.
                let _ = delete_refresh();
                self.clear().await;
                Err(Error::Unauthorized)
            }
            Err(e) => Err(e),
        }
    }

    async fn find_file(&self, http: &reqwest::Client, token: &str) -> Result<Option<String>> {
        if let Some(id) = self.file_id.lock().await.clone() {
            return Ok(Some(id));
        }
        #[derive(Deserialize)]
        struct List {
            files: Vec<File>,
        }
        #[derive(Deserialize)]
        struct File {
            id: String,
        }
        let list: List = drive_answer(
            http.get(format!(
                "{FILES_URL}?{}",
                encoded(&[("spaces", "appDataFolder"), ("q", &format!("name = '{FILE_NAME}'")), ("fields", "files(id)")])
            ))
            .bearer_auth(token)
            .send()
            .await,
        )
        .await?
        .json()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;
        let id = list.files.into_iter().next().map(|f| f.id);
        *self.file_id.lock().await = id.clone();
        Ok(id)
    }

    /// The places kept in your account (empty when there are none yet).
    pub async fn download(&self) -> Result<Places> {
        let token = self.token().await?;
        let http = http()?;
        let Some(id) = self.find_file(&http, &token).await? else { return Ok(Places::default()) };
        let res = http
            .get(format!("{FILES_URL}/{id}?alt=media"))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;
        if res.status().as_u16() == 404 {
            *self.file_id.lock().await = None;
            return Ok(Places::default());
        }
        let bytes = drive_answer(Ok(res)).await?.bytes().await.map_err(|e| Error::Network(e.to_string()))?;
        // An unreadable file (another version's, or damaged) starts over rather than failing.
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    /// Keeps `places` in your account (replacing what's there).
    pub async fn upload(&self, places: &Places) -> Result<()> {
        let token = self.token().await?;
        let http = http()?;
        let body = serde_json::to_vec(places).map_err(|e| Error::Config(e.to_string()))?;
        if let Some(id) = self.find_file(&http, &token).await? {
            drive_answer(
                http.patch(format!("{UPLOAD_URL}/{id}?uploadType=media"))
                    .bearer_auth(&token)
                    .header("content-type", "application/json")
                    .body(body)
                    .send()
                    .await,
            )
            .await?;
            return Ok(());
        }
        let boundary = format!("banime{}", URL_SAFE_NO_PAD.encode(rand::random::<[u8; 12]>()));
        let meta = serde_json::json!({ "name": FILE_NAME, "parents": ["appDataFolder"] });
        let mut multipart = format!("--{boundary}\r\ncontent-type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\ncontent-type: application/json\r\n\r\n").into_bytes();
        multipart.extend_from_slice(&body);
        multipart.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        #[derive(Deserialize)]
        struct Created {
            id: String,
        }
        let created: Created = drive_answer(
            http.post(format!("{UPLOAD_URL}?uploadType=multipart&fields=id"))
                .bearer_auth(&token)
                .header("content-type", format!("multipart/related; boundary={boundary}"))
                .body(multipart)
                .send()
                .await,
        )
        .await?
        .json()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;
        *self.file_id.lock().await = Some(created.id);
        Ok(())
    }
}

// ── What's kept, and merging ──────────────────────────────────────────────────────────────────

/// Your place in one show: its latest episode, the position in it, and when (Unix seconds).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Place {
    pub episode: i64,
    pub position: i64,
    #[serde(default)]
    pub length: Option<i64>,
    pub at: i64,
}

/// One place per show (by AniList id).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Places {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub shows: BTreeMap<i64, Place>,
}

/// The newer place for each show of the two (this device's latest, and your account's).
pub fn merge(local: &Places, remote: &Places) -> Places {
    let mut out = remote.clone();
    out.version = 1;
    for (id, mine) in &local.shows {
        match out.shows.get(id) {
            Some(theirs) if theirs.at >= mine.at => {}
            _ => {
                out.shows.insert(*id, mine.clone());
            }
        }
    }
    out
}

/// This device's latest place in each show (its most recently saved episode), from the
/// `playback` table.
pub async fn local_places(db: &crate::db::Db) -> Result<Places> {
    let rows: Vec<(i64, i64, i64, Option<i64>, i64)> = sqlx::query_as(
        "SELECT p.media_id, p.episode, p.position_s, p.duration_s, CAST(strftime('%s', p.updated_at) AS INTEGER)
         FROM playback p
         WHERE NOT EXISTS (SELECT 1 FROM playback q WHERE q.media_id = p.media_id AND q.updated_at > p.updated_at)",
    )
    .fetch_all(db)
    .await?;
    let mut places = Places { version: 1, ..Default::default() };
    for (media, episode, position, length, at) in rows {
        places.shows.insert(media, Place { episode, position, length, at });
    }
    Ok(places)
}

/// One sync: your account's places and this device's, the newer of each kept in both. A place
/// from another device becomes this device's saved position for that episode (so the player
/// resumes there). Returns whether anything came from another device.
pub async fn sync(db: &crate::db::Db, session: &Session) -> Result<bool> {
    let remote = session.download().await?;
    let local = local_places(db).await?;
    let merged = merge(&local, &remote);
    let mut came = false;
    for (media, place) in &merged.shows {
        if local.shows.get(media).is_some_and(|mine| mine.at >= place.at) {
            continue;
        }
        // Only for shows this device knows (the position row refers to the show's row).
        let done = sqlx::query(
            "INSERT INTO playback (media_id, episode, position_s, duration_s, updated_at)
             SELECT ?, ?, ?, ?, datetime(?, 'unixepoch') WHERE EXISTS (SELECT 1 FROM media WHERE id = ?)
             ON CONFLICT(media_id, episode) DO UPDATE SET position_s = excluded.position_s,
               duration_s = excluded.duration_s, updated_at = excluded.updated_at",
        )
        .bind(media)
        .bind(place.episode)
        .bind(place.position)
        .bind(place.length)
        .bind(place.at)
        .bind(media)
        .execute(db)
        .await?;
        came |= done.rows_affected() > 0;
    }
    if merged != remote {
        session.upload(&merged).await?;
    }
    Ok(came)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(episode: i64, position: i64, at: i64) -> Place {
        Place { episode, position, length: Some(1440), at }
    }

    #[test]
    fn the_newer_place_wins_per_show() {
        let mut local = Places::default();
        local.shows.insert(1, place(5, 300, 200));
        local.shows.insert(2, place(1, 10, 100));
        let mut remote = Places::default();
        remote.shows.insert(1, place(4, 900, 150));
        remote.shows.insert(2, place(3, 50, 300));
        remote.shows.insert(3, place(7, 70, 50));
        let m = merge(&local, &remote);
        assert_eq!(m.shows[&1], place(5, 300, 200), "this device's is newer");
        assert_eq!(m.shows[&2], place(3, 50, 300), "the account's is newer");
        assert_eq!(m.shows[&3], place(7, 70, 50), "only in the account");
    }

    #[test]
    fn the_drive_permission_is_looked_for_in_what_was_allowed() {
        assert!(allows_drive("openid https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/userinfo.email"));
        assert!(!allows_drive("openid https://www.googleapis.com/auth/userinfo.email"));
        assert!(!allows_drive("https://www.googleapis.com/auth/drive.appdata.extra"));
    }

    #[test]
    fn an_id_token_gives_the_email() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"someone@example.com"}"#);
        assert_eq!(email_of(&format!("x.{payload}.y")).as_deref(), Some("someone@example.com"));
        assert_eq!(email_of("not a token"), None);
    }
}
