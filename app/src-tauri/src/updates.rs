//! Automatic updates: checks GitHub Releases at launch and every 6 hours, downloads and verifies
//! the new setup in the background, and offers "Restart to update".

use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use banime_core::db;
use banime_core::updater::{self, Update};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

const UPDATE_AVAILABLE: &str = "update-available";
const UPDATE_READY: &str = "update-ready";
const UPDATE_FAILED: &str = "update-failed";
/// Soon after launch (once startup work has settled), so a new version is announced right away.
const FIRST_CHECK: Duration = Duration::from_secs(3);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// "1" when the user takes beta versions (GitHub pre-releases) as well as stable ones.
const BETA_PREF: &str = "updates.beta";

#[derive(Default)]
pub struct Updates {
    available: StdMutex<Option<Update>>,
    ready: StdMutex<Option<PathBuf>>,
    error: StdMutex<Option<String>>,
    checking: tokio::sync::Mutex<()>,
}

/// Development builds never update themselves. Windows installs the setup; Android the APK.
fn enabled() -> bool {
    !cfg!(debug_assertions)
}

async fn beta(state: &AppState) -> bool {
    // Same key as BETA_PREF (the query must be a literal).
    db::scalar_text(&state.db, "SELECT value FROM setting WHERE key = 'updates.beta'")
        .await
        .ok()
        .flatten()
        .is_some_and(|v| v == "1")
}

fn current(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

pub fn start(app: &AppHandle) {
    // The update that brought us here (and anything older) is installed: free the space.
    let dir = app.state::<AppState>().paths.cache.join("updates");
    let removed = updater::clean_downloads(&dir, &current(app));
    if removed > 0 {
        tracing::info!(removed, "deleted installed update downloads");
    }
    if !enabled() {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            check_and_download(&handle).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

async fn check_and_download(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(_guard) = state.updates.checking.try_lock() else { return };
    let set_error = |e: Option<String>| {
        if let Ok(mut slot) = state.updates.error.lock() {
            *slot = e;
        }
    };
    let update = match updater::check(&current(app), beta(&state).await).await {
        Ok(Some(u)) => u,
        Ok(None) => return set_error(None),
        Err(e) => {
            tracing::warn!(error = %e, "update check failed");
            return set_error(Some(e.to_string()));
        }
    };
    let previous = state.updates.available.lock().ok().and_then(|mut slot| slot.replace(update.clone()));
    // This version was already downloaded on an earlier check: nothing new to announce or fetch.
    let ready = state.updates.ready.lock().map(|r| r.is_some()).unwrap_or(false);
    if ready && previous.is_some_and(|p| p.version == update.version) {
        return;
    }
    // Announced before the download, so the user hears about it at once.
    let _ = app.emit(UPDATE_AVAILABLE, &update);
    match updater::download(&update, &state.paths.cache.join("updates")).await {
        Ok(path) => {
            tracing::info!(version = update.version, "update downloaded and verified");
            // Only this one waits to be installed: earlier downloads (an older beta) go.
            let removed = updater::keep_only(&state.paths.cache.join("updates"), &path);
            if removed > 0 {
                tracing::info!(removed, "deleted replaced update downloads");
            }
            if let Ok(mut slot) = state.updates.ready.lock() {
                *slot = Some(path);
            }
            set_error(None);
            let _ = app.emit(UPDATE_READY, &update);
            notify_ready(app, &update);
        }
        Err(e) => {
            tracing::warn!(error = %e, "update download failed");
            set_error(Some(e.to_string()));
            let _ = app.emit(UPDATE_FAILED, &update);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    current: String,
    enabled: bool,
    /// Takes beta versions too.
    beta: bool,
    available: Option<Update>,
    ready: bool,
    error: Option<String>,
}

#[tauri::command]
pub async fn update_status(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<UpdateStatus, String> {
    Ok(status(&app, &state).await)
}

async fn status(app: &AppHandle, state: &AppState) -> UpdateStatus {
    UpdateStatus {
        current: current(app),
        enabled: enabled(),
        beta: beta(state).await,
        available: state.updates.available.lock().ok().and_then(|u| u.clone()),
        ready: state.updates.ready.lock().map(|r| r.is_some()).unwrap_or(false),
        error: state.updates.error.lock().ok().and_then(|e| e.clone()),
    }
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<UpdateStatus, String> {
    if enabled() {
        check_and_download(&app).await;
    }
    Ok(status(&app, &state).await)
}

/// Switches between stable and beta versions, then checks again on the new channel. A version
/// found on the old channel is forgotten (leaving beta drops a beta that wasn't installed yet).
/// Nothing is ever downgraded: an installed beta stays until stable passes it.
#[tauri::command]
pub async fn set_update_channel(app: AppHandle, state: tauri::State<'_, AppState>, beta: bool) -> Result<UpdateStatus, String> {
    db::set_setting(&state.db, BETA_PREF, if beta { "1" } else { "0" }).await.map_err(crate::msg)?;
    {
        let _guard = state.updates.checking.lock().await;
        if let Ok(mut s) = state.updates.available.lock() {
            *s = None;
        }
        if let Ok(mut s) = state.updates.ready.lock() {
            *s = None;
        }
        if let Ok(mut s) = state.updates.error.lock() {
            *s = None;
        }
        // What the old channel downloaded isn't wanted now (a beta, after leaving Beta): the
        // check below fetches the new channel's own update, if there is one.
        let removed = updater::keep_only(&state.paths.cache.join("updates"), std::path::Path::new(""));
        if removed > 0 {
            tracing::info!(removed, "deleted the other channel's update downloads");
        }
    }
    if enabled() {
        check_and_download(&app).await;
    }
    Ok(status(&app, &state).await)
}

/// "0.4.5-beta.2" as people read it: "0.4.5 Beta 2".
fn version_label(v: &str) -> String {
    match v.split_once("-beta.") {
        Some((base, n)) => format!("{base} Beta {n}"),
        None => v.to_owned(),
    }
}

/// The system notification for a downloaded update: clicking it (Windows) restarts into the
/// update; tapping it (Android) opens the installer, even with B-Anime closed. The in-app notice
/// stays either way.
fn notify_ready(app: &AppHandle, update: &Update) {
    let title = format!("B-Anime {} is ready", version_label(&update.version));
    #[cfg(target_os = "android")]
    {
        let Some(path) = app.state::<AppState>().updates.ready.lock().ok().and_then(|r| r.clone()) else { return };
        let authority = format!("{}.fileprovider", app.config().identifier);
        if let Err(e) = crate::android_install::notify_update_ready(&path, &authority, &title, "Tap to install it.") {
            tracing::warn!(error = %e, "update notification failed");
        }
    }
    #[cfg(windows)]
    {
        use tauri_winrt_notification::Toast;
        let id = app.config().identifier.clone();
        if let Err(e) = register_toast_app(app, &id) {
            tracing::warn!(error = %e, "couldn't register for notifications");
        }
        let handle = app.clone();
        let shown = Toast::new(&id)
            .title(&title)
            .text1("Click to restart B-Anime and update.")
            .on_activated(move |_| {
                if let Err(e) = install_now(&handle) {
                    tracing::warn!(error = %e, "update from the notification failed");
                }
                Ok(())
            })
            .show();
        if let Err(e) = shown {
            tracing::warn!(error = %e, "update notification failed");
        }
    }
    #[cfg(not(any(windows, target_os = "android")))]
    let _ = (app, title);
}

/// Windows shows a notification under the name and icon registered for its app id: B-Anime's
/// installer adds no such entry, so it's (re)written here (for this user) before each one.
#[cfg(windows)]
fn register_toast_app(app: &AppHandle, id: &str) -> Result<(), String> {
    let icon = app.state::<AppState>().paths.cache.join("notification-icon.png");
    if !icon.exists() {
        std::fs::write(&icon, include_bytes!("../icons/128x128.png")).map_err(|e| e.to_string())?;
    }
    let key = windows_registry::CURRENT_USER
        .create(format!(r"Software\Classes\AppUserModelId\{id}"))
        .map_err(|e| e.to_string())?;
    key.set_string("DisplayName", "B-Anime").map_err(|e| e.to_string())?;
    key.set_string("IconUri", icon.to_string_lossy()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Desktop: runs the verified installer in update mode and quits; the installer relaunches
/// B-Anime. Android: opens the system installer on the verified APK; Android replaces the app
/// once the user confirms, so B-Anime keeps running until then.
#[tauri::command]
pub fn install_update(app: AppHandle) -> Result<(), String> {
    install_now(&app)
}

fn install_now(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let path = state.updates.ready.lock().ok().and_then(|r| r.clone()).ok_or("No update is ready yet.")?;
    #[cfg(target_os = "android")]
    {
        let authority = format!("{}.fileprovider", app.config().identifier);
        return crate::android_install::install_apk(&path, &authority);
    }
    #[cfg(not(target_os = "android"))]
    {
        updater::launch_installer(&path).map_err(crate::msg)?;
        app.exit(0);
        Ok(())
    }
}
