#[cfg(target_os = "android")]
mod android_install;
mod library;
mod playback;
mod updates;
mod watch;
mod whats_new;

use std::sync::Arc;
use std::time::Duration;

use banime_core::anilist::{self, Viewer, auth, browse, media, sync::SyncReport};
use banime_core::{AppPaths, Error, db, extensions, home, streaming};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, Notify};
use tracing_subscriber::EnvFilter;

/// Tells the UI to refetch anything built from the library or lists.
pub(crate) const LIBRARY_CHANGED: &str = "library-changed";

pub(crate) struct AppState {
    db: db::Db,
    paths: AppPaths,
    /// Library folders: scans, the watcher, scan progress.
    library: library::Library,
    updates: updates::Updates,
    watch: watch::Watch,
    /// Held while a sync runs; a second sync request waits for the first instead of racing it.
    sync_lock: Mutex<()>,
    /// Wakes a sign-in that's waiting on the browser, to cancel it.
    cancel_login: Arc<Notify>,
    /// Why the last automatic sync failed, until one succeeds. Shown in the UI.
    sync_error: std::sync::Mutex<Option<String>>,
    /// Loopback proxy every stream, subtitle and manga page goes through.
    proxy: streaming::Proxy,
    item_cache: std::sync::Mutex<ItemCache>,
}

type ItemCache = std::collections::HashMap<(String, String), (std::time::Instant, Vec<streaming::SourceItem>)>;

/// Syncing is automatic: at launch, on this timer, and from the UI's triggers (window focus
/// after 5+ minutes away, network back online). A failing sync retries on every trigger.
const PERIODIC_SYNC: Duration = Duration::from_secs(15 * 60);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    /// False when this build has no AniList client id; sign-in can't work.
    client_configured: bool,
    viewer: Option<Viewer>,
    last_synced: Option<String>,
    /// Set while automatic syncing is failing (e.g. offline); it keeps retrying.
    sync_error: Option<String>,
}

pub(crate) fn msg(e: Error) -> String {
    e.to_string()
}

#[tauri::command]
async fn get_home(state: tauri::State<'_, AppState>) -> Result<home::HomeData, String> {
    home::load(&state.db).await.map_err(msg)
}

#[tauri::command]
async fn get_library(
    state: tauri::State<'_, AppState>,
    kind: banime_core::library::Kind,
) -> Result<Vec<banime_core::library::LibraryEntry>, String> {
    banime_core::library::list(&state.db, kind).await.map_err(msg)
}

#[tauri::command]
async fn get_media(state: tauri::State<'_, AppState>, id: i64) -> Result<media::MediaPage, String> {
    // Signed-in requests aren't needed for public detail; the token only adds rate headroom.
    let token = auth::load_token().ok().flatten();
    let client = anilist::Client::new(token).map_err(msg)?;
    media::page(&client, &state.db, id).await.map_err(msg)
}

#[tauri::command]
async fn browse(state: tauri::State<'_, AppState>, filter: browse::BrowseFilter) -> Result<browse::BrowsePage, String> {
    let client = anilist::Client::new(auth::load_token().ok().flatten()).map_err(msg)?;
    browse::browse(&client, &state.db, filter).await.map_err(msg)
}

#[tauri::command]
async fn get_schedule(
    state: tauri::State<'_, AppState>,
    scope: browse::Scope,
    from: i64,
    to: i64,
) -> Result<Vec<browse::ScheduleItem>, String> {
    let client = anilist::Client::new(auth::load_token().ok().flatten()).map_err(msg)?;
    browse::schedule(&client, &state.db, scope, from, to).await.map_err(msg)
}

// ── Streaming & reading ──────────────────────────────────────────────────────

#[tauri::command]
async fn find_source(
    state: tauri::State<'_, AppState>,
    media_id: i64,
    ext: String,
    titles: Vec<String>,
    year: Option<i64>,
    dub: Option<bool>,
) -> Result<streaming::MatchState, String> {
    streaming::find_match(&state.db, &state.paths.extensions, media_id, &ext, &titles, year, dub.unwrap_or(false))
        .await
        .map_err(msg)
}

#[tauri::command]
async fn search_source(
    state: tauri::State<'_, AppState>,
    ext: String,
    query: String,
    dub: Option<bool>,
) -> Result<Vec<streaming::SourceResult>, String> {
    streaming::search(&state.db, &state.paths.extensions, &ext, &query, dub.unwrap_or(false), None).await.map_err(msg)
}

#[tauri::command]
async fn set_source_match(
    state: tauri::State<'_, AppState>,
    media_id: i64,
    ext: String,
    source_id: String,
    title: String,
    dub: Option<bool>,
) -> Result<(), String> {
    streaming::save_match(&state.db, media_id, &ext, dub.unwrap_or(false), &source_id, &title, false).await.map_err(msg)
}

#[tauri::command]
async fn clear_source_match(state: tauri::State<'_, AppState>, media_id: i64, ext: String, dub: Option<bool>) -> Result<(), String> {
    streaming::clear_match(&state.db, media_id, &ext, dub.unwrap_or(false)).await.map_err(msg)
}

/// Episodes or chapters on a source. Cached for 10 minutes: the player and reader ask often.
#[tauri::command]
async fn source_items(
    state: tauri::State<'_, AppState>,
    ext: String,
    source_id: String,
    refresh: Option<bool>,
) -> Result<Vec<streaming::SourceItem>, String> {
    let key = (ext.clone(), source_id.clone());
    if !refresh.unwrap_or(false) {
        if let Some((at, items)) = state.item_cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
            if at.elapsed() < Duration::from_secs(600) {
                return Ok(items);
            }
        }
    }
    let installed = extensions::get(&state.db, &ext).await.map_err(msg)?.ok_or("That extension isn't installed.")?;
    let items = streaming::items(&state.db, &state.paths.extensions, &ext, installed.manifest.kind, &source_id)
        .await
        .map_err(msg)?;
    if let Ok(mut c) = state.item_cache.lock() {
        c.insert(key, (std::time::Instant::now(), items.clone()));
    }
    Ok(items)
}

#[tauri::command]
async fn get_stream(state: tauri::State<'_, AppState>, ext: String, episode_id: String) -> Result<streaming::Stream, String> {
    streaming::stream(&state.db, &state.paths.extensions, &state.proxy, &ext, &episode_id).await.map_err(msg)
}

#[tauri::command]
async fn get_pages(state: tauri::State<'_, AppState>, ext: String, chapter_id: String) -> Result<Vec<String>, String> {
    streaming::pages(&state.db, &state.paths.extensions, &state.proxy, &ext, &chapter_id).await.map_err(msg)
}

#[tauri::command]
async fn get_skip_times(mal_id: i64, episode: i64, length: f64) -> Result<Vec<streaming::SkipTime>, String> {
    // Best effort: no skip buttons is fine, an error dialog is not.
    Ok(streaming::skip_times(mal_id, episode, length).await.unwrap_or_default())
}

#[tauri::command]
async fn save_position(
    state: tauri::State<'_, AppState>,
    media_id: i64,
    episode: i64,
    position: f64,
    duration: f64,
) -> Result<(), String> {
    streaming::save_position(&state.db, media_id, episode, position, duration).await.map_err(msg)
}

#[tauri::command]
async fn get_position(state: tauri::State<'_, AppState>, media_id: i64, episode: i64) -> Result<Option<i64>, String> {
    streaming::position(&state.db, media_id, episode).await.map_err(msg)
}

// ── Extensions ───────────────────────────────────────────────────────────────

#[tauri::command]
async fn list_extensions(state: tauri::State<'_, AppState>) -> Result<Vec<extensions::Installed>, String> {
    extensions::list(&state.db).await.map_err(msg)
}

/// Downloads and checks an extension so the UI can show what it is before installing.
#[tauri::command]
async fn preview_extension(url: String) -> Result<extensions::Manifest, String> {
    extensions::fetch(&url, extensions::Limits::default()).await.map(|(m, _)| m).map_err(msg)
}

#[tauri::command]
async fn install_extension(state: tauri::State<'_, AppState>, url: String) -> Result<extensions::Installed, String> {
    extensions::install(&state.db, &state.paths.extensions, &url, extensions::Limits::default())
        .await
        .map_err(msg)
}

#[tauri::command]
async fn update_extension(state: tauri::State<'_, AppState>, id: String) -> Result<extensions::Installed, String> {
    let ext = extensions::get(&state.db, &id).await.map_err(msg)?.ok_or("Not installed.")?;
    // Installed from disk in a development build: "update" re-reads the files.
    if let Some(path) = extensions::local_source(&ext) {
        if !LOCAL_EXTENSIONS {
            return Err(LOCAL_ONLY_IN_DEV.into());
        }
        return extensions::install_local(&state.db, &state.paths.extensions, &path, extensions::Limits::default())
            .await
            .map_err(msg);
    }
    extensions::install(&state.db, &state.paths.extensions, &ext.source_url, extensions::Limits::default())
        .await
        .map_err(msg)
}

/// Installing extensions from files on this computer is for testing your own extension, so it
/// exists in development builds (`tauri dev`) only. Release builds keep installs to http(s).
const LOCAL_EXTENSIONS: bool = cfg!(debug_assertions);
const LOCAL_ONLY_IN_DEV: &str = "Installing extensions from a file is only available in development builds.";

/// Development builds only: pick an extension manifest on disk and install it.
#[tauri::command]
async fn install_local_extension(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<extensions::Installed>, String> {
    if !LOCAL_EXTENSIONS {
        return Err(LOCAL_ONLY_IN_DEV.into());
    }
    use tauri_plugin_dialog::DialogExt;
    #[cfg(desktop)]
    let window = app.get_webview_window("main").ok_or("The main window isn't open.")?;
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = handle
            .dialog()
            .file()
            .set_title("Install an extension from a manifest")
            .add_filter("Extension manifest", &["json"]);
        #[cfg(desktop)]
        let dialog = dialog.set_parent(&window);
        dialog.blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    extensions::install_local(&state.db, &state.paths.extensions, &path, extensions::Limits::default())
        .await
        .map(Some)
        .map_err(msg)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionUpdate {
    id: String,
    version: String,
}

const EXTENSIONS_UPDATED: &str = "extensions-updated";

/// One extension updated at launch, for the UI's notice.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UpdatedExtension {
    id: String,
    name: String,
    version: String,
}

/// At launch: installs the newer version of every installed extension whose source offers one,
/// then tells the UI which were updated. Extensions installed from a file (development) are left
/// alone, and an unreachable source or a failed install just keeps the installed version.
async fn auto_update_extensions(app: &AppHandle) {
    // Let startup (window, first sync) go first.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let state = app.state::<AppState>();
    let Ok(installed) = extensions::list(&state.db).await else { return };
    let mut updated = Vec::new();
    for ext in installed {
        if extensions::local_source(&ext).is_some() {
            continue;
        }
        let Ok(Some(version)) = extensions::available_update(&ext, extensions::Limits::default()).await else {
            continue;
        };
        match extensions::install(&state.db, &state.paths.extensions, &ext.source_url, extensions::Limits::default()).await {
            Ok(new) => {
                tracing::info!(id = %ext.manifest.id, from = %ext.manifest.version, to = %version, "extension updated");
                updated.push(UpdatedExtension { id: new.manifest.id, name: new.manifest.name, version: new.manifest.version });
            }
            Err(e) => tracing::warn!(id = %ext.manifest.id, error = %e, "extension update failed"),
        }
    }
    if !updated.is_empty() {
        let _ = app.emit(EXTENSIONS_UPDATED, &updated);
    }
}

/// Checks every installed extension's source for a newer version. Unreachable sources are skipped.
#[tauri::command]
async fn check_extension_updates(state: tauri::State<'_, AppState>) -> Result<Vec<ExtensionUpdate>, String> {
    let mut updates = Vec::new();
    for ext in extensions::list(&state.db).await.map_err(msg)? {
        if let Ok(Some(version)) = extensions::available_update(&ext, extensions::Limits::default()).await {
            updates.push(ExtensionUpdate { id: ext.manifest.id, version });
        }
    }
    Ok(updates)
}

#[tauri::command]
async fn set_extension_enabled(state: tauri::State<'_, AppState>, id: String, enabled: bool) -> Result<(), String> {
    extensions::set_enabled(&state.db, &id, enabled).await.map_err(msg)
}

#[tauri::command]
async fn remove_extension(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    extensions::remove(&state.db, &state.paths.extensions, &id).await.map_err(msg)
}

/// The Extensions screen's tester: runs `search` and returns the result with the logs.
#[tauri::command]
async fn try_extension(
    state: tauri::State<'_, AppState>,
    id: String,
    query: String,
) -> Result<extensions::CallOutput, String> {
    let args = serde_json::json!([{ "query": query }]);
    extensions::call(&state.db, &state.paths.extensions, &id, "search", &args, extensions::Limits::default())
        .await
        .map_err(msg)
}

/// B-Anime's own repository, shown in its own panel apart from the user's list.
#[tauri::command]
fn official_extension_repo() -> &'static str {
    extensions::OFFICIAL_REPO
}

const REPOS_KEY: &str = "extensions.repos";

/// The user's repositories. Until they change the list, it holds B-Anime's own examples.
#[tauri::command]
async fn get_extension_repos(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let raw = sqlx_scalar(&state.db, "SELECT value FROM setting WHERE key = 'extensions.repos'").await.map_err(msg)?;
    Ok(match raw.and_then(|r| serde_json::from_str::<Vec<String>>(&r).ok()) {
        Some(list) => list,
        None => vec![extensions::DEFAULT_REPO.to_owned()],
    })
}

#[tauri::command]
async fn set_extension_repos(state: tauri::State<'_, AppState>, repos: Vec<String>) -> Result<(), String> {
    let mut clean: Vec<String> = repos.into_iter().map(|r| r.trim().to_owned()).filter(|r| !r.is_empty()).collect();
    clean.dedup();
    banime_core::db::set_setting(&state.db, REPOS_KEY, &serde_json::to_string(&clean).map_err(|e| e.to_string())?)
        .await
        .map_err(msg)
}

#[tauri::command]
async fn read_extension_repo(url: String) -> Result<extensions::Repo, String> {
    extensions::read_repo(&url, extensions::Limits::default()).await.map_err(msg)
}

/// Opens a show on anilist.co. Takes an id, not a URL, so the UI can't open arbitrary links.
#[tauri::command]
fn open_anilist(app: AppHandle, id: u64, kind: banime_core::library::Kind) -> Result<(), String> {
    let path = match kind {
        banime_core::library::Kind::Anime => "anime",
        banime_core::library::Kind::Manga => "manga",
    };
    app.opener()
        .open_url(format!("https://anilist.co/{path}/{id}"), None::<&str>)
        .map_err(|e| e.to_string())
}

/// The project's Patreon page (Support in the sidebar).
const SUPPORT_URL: &str = "https://www.patreon.com/cw/ahmedanjum30";

/// Opens the Patreon page in the browser. The address is fixed here, not passed in.
#[tauri::command]
fn open_support(app: AppHandle) -> Result<(), String> {
    app.opener().open_url(SUPPORT_URL, None::<&str>).map_err(|e| e.to_string())
}

/// Where the Android APK is published (Android app in the sidebar).
const ANDROID_URL: &str = "https://github.com/ahmedanjum30/B-Anime/releases/latest";

/// Opens the latest release (with the Android APK) in the browser. The address is fixed here.
#[tauri::command]
fn open_android_download(app: AppHandle) -> Result<(), String> {
    app.opener().open_url(ANDROID_URL, None::<&str>).map_err(|e| e.to_string())
}

/// Saves a list change to AniList and the local mirror; the UI refreshes via LIBRARY_CHANGED.
#[tauri::command]
async fn save_entry(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    media_id: i64,
    change: media::EntryChange,
) -> Result<media::ListEntry, String> {
    let token = auth::load_token().map_err(msg)?.ok_or_else(|| msg(Error::Unauthorized))?;
    let client = anilist::Client::new(Some(token)).map_err(msg)?;
    let entry = media::save_entry(&client, &state.db, media_id, change).await.map_err(msg)?;
    let _ = app.emit(LIBRARY_CHANGED, ());
    Ok(entry)
}

/// Removes a show from the user's AniList list and the local mirror.
#[tauri::command]
async fn delete_entry(app: AppHandle, state: tauri::State<'_, AppState>, media_id: i64) -> Result<(), String> {
    let token = auth::load_token().map_err(msg)?.ok_or_else(|| msg(Error::Unauthorized))?;
    let client = anilist::Client::new(Some(token)).map_err(msg)?;
    media::delete_entry(&client, &state.db, media_id).await.map_err(msg)?;
    let _ = app.emit(LIBRARY_CHANGED, ());
    Ok(())
}

#[tauri::command]
async fn get_account(state: tauri::State<'_, AppState>) -> Result<Account, String> {
    account(&state).await.map_err(msg)
}

async fn account(state: &AppState) -> banime_core::Result<Account> {
    // The saved profile only counts while there's a token to go with it; without one, nothing
    // would sync, so show the account as signed out instead of a stale profile.
    let viewer = match auth::load_token() {
        Ok(None) => None,
        _ => anilist::sync::stored_viewer(&state.db).await?,
    };
    let last_synced: Option<String> =
        sqlx_scalar(&state.db, "SELECT value FROM setting WHERE key = 'anilist.last_synced'").await?;
    let sync_error = state.sync_error.lock().ok().and_then(|e| e.clone());
    Ok(Account { client_configured: auth::client_id().is_some(), viewer, last_synced, sync_error })
}

async fn sqlx_scalar(db: &db::Db, sql: &'static str) -> banime_core::Result<Option<String>> {
    banime_core::db::scalar_text(db, sql).await
}

/// Opens AniList in the browser and waits (up to 5 minutes) for the user to approve.
#[tauri::command]
async fn anilist_login(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Account, String> {
    let client_id = auth::client_id().ok_or("This build has no AniList client id configured.")?;
    // A sign-in still waiting holds the callback port. That happens when the page was reloaded
    // (on phones, coming back from the browser can do that) and Connect was tapped again: the new
    // attempt replaces it. start_login waits for the port to come free.
    state.cancel_login.notify_waiters();
    let pending = auth::start_login(&client_id).await.map_err(msg)?;
    app.opener()
        .open_url(&pending.url, None::<&str>)
        .map_err(|e| format!("Couldn't open your browser: {e}"))?;

    let cancel = state.cancel_login.clone();
    let token = tokio::select! {
        t = pending.wait() => t.map_err(msg)?,
        _ = cancel.notified() => return Err("Sign-in cancelled.".into()),
    };

    // Kept straight away: the token is good whatever happens next.
    auth::save_token(&token).map_err(msg)?;
    let client = anilist::Client::new(Some(token.clone())).map_err(msg)?;
    // The token arrives while the browser is in front. On a phone, an app in the background may
    // have no network (Android blocks it), so the first request failed ("error sending request")
    // whenever logging in to AniList took a while. Network errors are retried for up to 2 minutes:
    // coming back to B-Anime lets them through.
    let mut tries = 0;
    let viewer = loop {
        match client.viewer().await {
            Ok(v) => break v,
            Err(banime_core::Error::Network(_)) if tries < 40 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(e) => return Err(msg(e)),
        }
    };
    run_sync(&app, &state, &client, &viewer).await.map_err(msg)?;
    if let Ok(mut e) = state.sync_error.lock() {
        *e = None;
    }
    account(&state).await.map_err(msg)
}

#[tauri::command]
fn anilist_cancel_login(state: tauri::State<'_, AppState>) {
    state.cancel_login.notify_waiters();
}

#[tauri::command]
async fn anilist_logout(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Account, String> {
    let _guard = state.sync_lock.lock().await;
    auth::delete_token().map_err(msg)?;
    anilist::sync::forget(&state.db).await.map_err(msg)?;
    if let Ok(mut e) = state.sync_error.lock() {
        *e = None;
    }
    let _ = app.emit(LIBRARY_CHANGED, ());
    account(&state).await.map_err(msg)
}

/// The UI's sync triggers (window focus, network back online). Syncs only when the last sync
/// is older than `min_age_secs`, and never runs two at once. Returns whether it synced.
#[tauri::command]
async fn sync_if_stale(app: AppHandle, state: tauri::State<'_, AppState>, min_age_secs: u64) -> Result<bool, String> {
    Ok(auto_sync(&app, &state, Duration::from_secs(min_age_secs)).await)
}

/// Syncs when signed in and the last sync is older than `min_age`. Failures are recorded for
/// the UI (and retried by the next trigger) rather than returned.
async fn auto_sync(app: &AppHandle, state: &AppState, min_age: Duration) -> bool {
    if state.sync_lock.try_lock().is_err() || !matches!(auth::load_token(), Ok(Some(_))) {
        return false;
    }
    let last: Option<String> = sqlx_scalar(&state.db, "SELECT value FROM setting WHERE key = 'anilist.last_synced'")
        .await
        .ok()
        .flatten();
    let fresh = last
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .is_some_and(|t| chrono::Utc::now().signed_duration_since(t).to_std().unwrap_or_default() < min_age);
    // A failing sync retries on every trigger, however recent the last success.
    let failing = state.sync_error.lock().map(|e| e.is_some()).unwrap_or(false);
    if fresh && !failing {
        return false;
    }

    let result = sync_with_stored_token(app, state).await;
    if let Ok(mut slot) = state.sync_error.lock() {
        *slot = result.as_ref().err().map(|e| e.to_string());
    }
    match result {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(error = %e, "automatic sync failed");
            let _ = app.emit(LIBRARY_CHANGED, ());
            false
        }
    }
}

async fn sync_with_stored_token(app: &AppHandle, state: &AppState) -> banime_core::Result<SyncReport> {
    let token = auth::load_token()?.ok_or(Error::Unauthorized)?;
    let client = anilist::Client::new(Some(token))?;
    let viewer = match client.viewer().await {
        Err(Error::Unauthorized) => {
            // Expired or revoked: drop the dead token so the UI offers sign-in again.
            auth::delete_token()?;
            anilist::sync::forget(&state.db).await?;
            let _ = app.emit(LIBRARY_CHANGED, ());
            return Err(Error::Unauthorized);
        }
        other => other?,
    };
    run_sync(app, state, &client, &viewer).await
}

async fn run_sync(
    app: &AppHandle,
    state: &AppState,
    client: &anilist::Client,
    viewer: &Viewer,
) -> banime_core::Result<SyncReport> {
    let _guard = state.sync_lock.lock().await;
    let report = anilist::sync::sync_all(client, &state.db, viewer).await?;
    let _ = app.emit(LIBRARY_CHANGED, ());
    Ok(report)
}

// On Android the app starts from this library (no main.rs), through the mobile entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let builder = tauri::Builder::default();
    // Desktop: one B-Anime at a time. Opening a banime:// link (an invite) while it runs hands
    // the link to the running copy (which receives it as a deep link) and brings it forward.
    // First, as the plugin asks.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }));
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // banime:// links (Watch together invites). Phones declare the scheme in the app's
            // manifest; Windows and Linux need it registered, which is redone at every launch so
            // it always points at this copy (installed or a development build).
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                if let Err(e) = app.deep_link().register_all() {
                    tracing::warn!(error = %e, "couldn't register banime:// links");
                }
            }
            // Phones have no OS config folder: use the app's private storage there.
            #[cfg(mobile)]
            let paths = AppPaths::resolve(Some(app.path().app_data_dir()?))?;
            // No credential store on phones: the AniList token lives in the app's private storage.
            #[cfg(mobile)]
            auth::use_token_file(paths.root.join("anilist-token"));
            #[cfg(desktop)]
            let paths = AppPaths::resolve(None)?;
            paths.ensure()?;
            tracing::info!(data_dir = %paths.root.display(), version = %app.package_info().version, "starting B-Anime");
            let db = tauri::async_runtime::block_on(db::open(&paths.db))?;
            let proxy = tauri::async_runtime::block_on(streaming::Proxy::start())?;
            app.manage(AppState {
                proxy,
                item_cache: std::sync::Mutex::new(Default::default()),
                db,
                paths,
                sync_lock: Mutex::new(()),
                cancel_login: Arc::new(Notify::new()),
                sync_error: std::sync::Mutex::new(None),
                library: library::Library::start(app.handle()),
                updates: updates::Updates::default(),
                watch: watch::Watch::default(),
            });
            updates::start(app.handle());

            // Open maximized. The config flag alone doesn't stick on every Windows setup.
            #[cfg(desktop)]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.maximize();
            }

            // Installed extensions update themselves at launch (alongside the sync below).
            let ext_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { auto_update_extensions(&ext_handle).await });

            // Sync at launch, then keep lists fresh while the app is open. The UI shows the
            // cached copy meanwhile and adds focus / back-online triggers.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                auto_sync(&handle, &state, Duration::ZERO).await;
                // Then the library: catch up on files changed while closed, and watch from now on.
                library::rewatch(&handle).await;
                library::run_scan(&handle).await;
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    auto_sync(&handle, &state, PERIODIC_SYNC).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_home,
            get_library,
            get_media,
            save_entry,
            delete_entry,
            install_local_extension,
            official_extension_repo,
            browse,
            get_schedule,
            find_source,
            search_source,
            set_source_match,
            clear_source_match,
            source_items,
            get_stream,
            get_pages,
            get_skip_times,
            save_position,
            get_position,
            playback::external_players,
            playback::open_external_player,
            playback::get_player_prefs,
            playback::set_player_pref,
            playback::set_languages,
            playback::pick_subtitle_file,
            playback::local_episodes,
            playback::local_stream,
            playback::play_local,
            library::library_status,
            library::pick_folder,
            library::add_library_root,
            library::remove_library_root,
            library::unmatched_files,
            library::assign_files,
            library::ignore_files,
            updates::update_status,
            updates::check_for_update,
            updates::install_update,
            updates::set_update_channel,
            watch::watch_host,
            watch::watch_join,
            watch::watch_rejoin,
            watch::watch_leave,
            watch::watch_room,
            watch::watch_send_state,
            watch::watch_send_status,
            watch::watch_ready,
            watch::watch_start,
            watch::watch_check,
            whats_new::whats_new,
            whats_new::whats_new_seen,
            list_extensions,
            preview_extension,
            install_extension,
            update_extension,
            check_extension_updates,
            set_extension_enabled,
            remove_extension,
            try_extension,
            get_extension_repos,
            set_extension_repos,
            read_extension_repo,
            open_anilist,
            open_support,
            open_android_download,
            get_account,
            anilist_login,
            anilist_cancel_login,
            anilist_logout,
            sync_if_stale
        ])
        .run(tauri::generate_context!())
        .expect("error while running B-Anime");
}
