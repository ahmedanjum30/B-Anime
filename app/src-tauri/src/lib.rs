use std::sync::Arc;

use banime_core::anilist::{self, Viewer, auth, sync::SyncReport};
use banime_core::{AppPaths, Error, db, home};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, Notify};
use tracing_subscriber::EnvFilter;

/// Tells the UI to refetch anything built from the library or lists.
const LIBRARY_CHANGED: &str = "library-changed";

struct AppState {
    db: db::Db,
    #[allow(dead_code)]
    paths: AppPaths,
    /// Held while a sync runs; a second sync request waits for the first instead of racing it.
    sync_lock: Mutex<()>,
    /// Wakes a sign-in that's waiting on the browser, to cancel it.
    cancel_login: Arc<Notify>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    /// False when this build has no AniList client id; sign-in can't work.
    client_configured: bool,
    viewer: Option<Viewer>,
    last_synced: Option<String>,
}

fn msg(e: Error) -> String {
    e.to_string()
}

#[tauri::command]
async fn get_home(state: tauri::State<'_, AppState>) -> Result<home::HomeData, String> {
    home::load(&state.db).await.map_err(msg)
}

#[tauri::command]
async fn get_account(state: tauri::State<'_, AppState>) -> Result<Account, String> {
    account(&state).await.map_err(msg)
}

async fn account(state: &AppState) -> banime_core::Result<Account> {
    let viewer = anilist::sync::stored_viewer(&state.db).await?;
    let last_synced: Option<String> =
        sqlx_scalar(&state.db, "SELECT value FROM setting WHERE key = 'anilist.last_synced'").await?;
    Ok(Account { client_configured: auth::client_id().is_some(), viewer, last_synced })
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
    let _ = app.emit(LIBRARY_CHANGED, ());
    account(&state).await.map_err(msg)
}

#[tauri::command]
async fn anilist_sync(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<SyncReport, String> {
    sync_with_stored_token(&app, &state).await.map_err(msg)
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
            app.manage(AppState {
                db,
                paths,
                sync_lock: Mutex::new(()),
                cancel_login: Arc::new(Notify::new()),
            });

            // Open maximized. The config flag alone doesn't stick on every Windows setup.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.maximize();
            }

            // Refresh lists in the background on launch; the UI shows the cached copy meanwhile.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                if matches!(auth::load_token(), Ok(Some(_))) {
                    if let Err(e) = sync_with_stored_token(&handle, &state).await {
                        tracing::warn!(error = %e, "startup sync failed");
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_home,
            get_account,
            anilist_login,
            anilist_cancel_login,
            anilist_logout,
            anilist_sync
        ])
        .run(tauri::generate_context!())
        .expect("error while running B-Anime");
}
