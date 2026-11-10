//! Automatic updates: checks GitHub Releases at launch and every 6 hours, downloads and verifies
//! the new setup in the background, and offers "Restart to update".

use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use banime_core::updater::{self, Update};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, msg};

const UPDATE_READY: &str = "update-ready";
const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Default)]
pub struct Updates {
    available: StdMutex<Option<Update>>,
    ready: StdMutex<Option<PathBuf>>,
    error: StdMutex<Option<String>>,
    checking: tokio::sync::Mutex<()>,
}

/// Development builds never update themselves.
fn enabled() -> bool {
    !cfg!(debug_assertions)
}

fn current(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

pub fn start(app: &AppHandle) {
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
    let update = match updater::check(&current(app)).await {
        Ok(Some(u)) => u,
        Ok(None) => return set_error(None),
        Err(e) => {
            tracing::warn!(error = %e, "update check failed");
            return set_error(Some(e.to_string()));
        }
    };
    if let Ok(mut slot) = state.updates.available.lock() {
        *slot = Some(update.clone());
    }
    match updater::download(&update, &state.paths.cache.join("updates")).await {
        Ok(path) => {
            tracing::info!(version = update.version, "update downloaded and verified");
            if let Ok(mut slot) = state.updates.ready.lock() {
                *slot = Some(path);
            }
            set_error(None);
            let _ = app.emit(UPDATE_READY, &update);
        }
        Err(e) => {
            tracing::warn!(error = %e, "update download failed");
            set_error(Some(e.to_string()));
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    current: String,
    enabled: bool,
    available: Option<Update>,
    ready: bool,
    error: Option<String>,
}

#[tauri::command]
pub fn update_status(app: AppHandle, state: tauri::State<'_, AppState>) -> UpdateStatus {
    UpdateStatus {
        current: current(&app),
        enabled: enabled(),
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
    Ok(update_status(app.clone(), state))
}

/// Runs the verified installer in update mode and quits; the installer relaunches B-Anime.
#[tauri::command]
pub fn install_update(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let path = state.updates.ready.lock().ok().and_then(|r| r.clone()).ok_or("No update is ready yet.")?;
    updater::launch_installer(&path).map_err(msg)?;
    app.exit(0);
    Ok(())
}
