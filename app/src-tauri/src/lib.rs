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
const LIBRARY_CHANGED: &str = "library-changed";

struct AppState {
    db: db::Db,
    paths: AppPaths,
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

fn msg(e: Error) -> String {
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
) -> Result<streaming::MatchState, String> {
    streaming::find_match(&state.db, &state.paths.extensions, media_id, &ext, &titles, year).await.map_err(msg)
}

#[tauri::command]
async fn search_source(state: tauri::State<'_, AppState>, ext: String, query: String) -> Result<Vec<streaming::SourceResult>, String> {
    streaming::search(&state.db, &state.paths.extensions, &ext, &query).await.map_err(msg)
}

#[tauri::command]
async fn set_source_match(
    state: tauri::State<'_, AppState>,
    media_id: i64,
    ext: String,
    source_id: String,
    title: String,
) -> Result<(), String> {
    streaming::save_match(&state.db, media_id, &ext, &source_id, &title, false).await.map_err(msg)
}

#[tauri::command]
async fn clear_source_match(state: tauri::State<'_, AppState>, media_id: i64, ext: String) -> Result<(), String> {
    streaming::clear_match(&state.db, media_id, &ext).await.map_err(msg)
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

/// MPV or VLC, from PATH or their usual install folders.
fn find_player(name: &str) -> Option<std::path::PathBuf> {
    let exe = format!("{name}.exe");
    let mut candidates: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(&exe)).collect())
        .unwrap_or_default();
    let env = |k: &str| std::env::var_os(k).map(std::path::PathBuf::from);
    match name {
        "mpv" => {
            for base in [env("ProgramFiles"), env("LOCALAPPDATA").map(|p| p.join("Programs"))].into_iter().flatten() {
                candidates.push(base.join("mpv").join(&exe));
            }
            if let Some(home) = env("USERPROFILE") {
                candidates.push(home.join("scoop").join("apps").join("mpv").join("current").join(&exe));
            }
        }
        "vlc" => {
            for base in [env("ProgramFiles"), env("ProgramFiles(x86)")].into_iter().flatten() {
                candidates.push(base.join("VideoLAN").join("VLC").join(&exe));
            }
        }
        _ => return None,
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[tauri::command]
fn external_players() -> Vec<String> {
    ["mpv", "vlc"].into_iter().filter(|p| find_player(p).is_some()).map(str::to_owned).collect()
}

/// Opens a proxied stream URL in MPV or VLC. Only the local proxy's URLs are accepted.
#[tauri::command]
fn open_external_player(
    state: tauri::State<'_, AppState>,
    player: String,
    url: String,
    title: String,
    subtitle: Option<String>,
) -> Result<(), String> {
    let origin = state.proxy.origin();
    if !url.starts_with(&origin) || subtitle.as_deref().is_some_and(|s| !s.starts_with(&origin)) {
        return Err("Only B-Anime streams can be opened externally.".into());
    }
    let exe = find_player(&player).ok_or_else(|| format!("{player} isn't installed."))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(&url);
    match player.as_str() {
        "mpv" => {
            cmd.arg(format!("--force-media-title={title}"));
            if let Some(s) = subtitle {
                cmd.arg(format!("--sub-file={s}"));
            }
        }
        _ => {
            cmd.arg(format!("--meta-title={title}"));
            if let Some(s) = subtitle {
                cmd.arg(format!("--input-slave={s}"));
            }
        }
    }
    cmd.spawn().map(|_| ()).map_err(|e| format!("Couldn't start {player}: {e}"))
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
    extensions::install(&state.db, &state.paths.extensions, &ext.source_url, extensions::Limits::default())
        .await
        .map_err(msg)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionUpdate {
    id: String,
    version: String,
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

const REPOS_KEY: &str = "extensions.repos";

#[tauri::command]
async fn get_extension_repos(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let raw = sqlx_scalar(&state.db, "SELECT value FROM setting WHERE key = 'extensions.repos'").await.map_err(msg)?;
    Ok(raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default())
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
async fn read_extension_repo(url: String) -> Result<Vec<extensions::RepoEntry>, String> {
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

#[tauri::command]
async fn get_account(state: tauri::State<'_, AppState>) -> Result<Account, String> {
    account(&state).await.map_err(msg)
}

async fn account(state: &AppState) -> banime_core::Result<Account> {
    let viewer = anilist::sync::stored_viewer(&state.db).await?;
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
    let pending = auth::start_login(&client_id).await.map_err(msg)?;
    app.opener()
        .open_url(&pending.url, None::<&str>)
        .map_err(|e| format!("Couldn't open your browser: {e}"))?;

    let cancel = state.cancel_login.clone();
    let token = tokio::select! {
        t = pending.wait() => t.map_err(msg)?,
        _ = cancel.notified() => return Err("Sign-in cancelled.".into()),
    };

    let client = anilist::Client::new(Some(token.clone())).map_err(msg)?;
    let viewer = client.viewer().await.map_err(msg)?;
    auth::save_token(&token).map_err(msg)?;
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

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let paths = AppPaths::resolve(None)?;
            paths.ensure()?;
            tracing::info!(data_dir = %paths.root.display(), "starting B-Anime");
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
            });

            // Open maximized. The config flag alone doesn't stick on every Windows setup.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.maximize();
            }

            // Sync at launch, then keep lists fresh while the app is open. The UI shows the
            // cached copy meanwhile and adds focus / back-online triggers.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                auto_sync(&handle, &state, Duration::ZERO).await;
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
            external_players,
            open_external_player,
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
            get_account,
            anilist_login,
            anilist_cancel_login,
            anilist_logout,
            sync_if_stale
        ])
        .run(tauri::generate_context!())
        .expect("error while running B-Anime");
}
