//! The local library in the app: folder management, automatic scans (launch, folder added,
//! files changed on disk), and the unmatched-files fixer.

use std::sync::Mutex as StdMutex;
use std::time::Duration;

use banime_core::anilist::{self, auth, media};
use banime_core::scanner::{self, ScanReport};
use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::{AppState, LIBRARY_CHANGED, msg};

const SCAN_PROGRESS: &str = "scan-progress";
/// Files usually arrive in bursts (a download finishing, a folder copied in): wait for quiet.
const SETTLE: Duration = Duration::from_secs(5);

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStatus {
    pub running: bool,
    pub phase: Option<String>,
    pub done: usize,
    pub total: usize,
}

pub struct Library {
    pub status: StdMutex<ScanStatus>,
    scan_lock: tokio::sync::Mutex<()>,
    rescan: mpsc::UnboundedSender<()>,
    watcher: StdMutex<Option<notify::RecommendedWatcher>>,
}

impl Library {
    /// Starts the debounced rescan loop. Call `request_scan` to ask for a scan.
    pub fn start(app: &AppHandle) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<()>();
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            while rx.recv().await.is_some() {
                // Coalesce everything that arrives while things settle.
                loop {
                    tokio::time::sleep(SETTLE).await;
                    let mut more = false;
                    while rx.try_recv().is_ok() {
                        more = true;
                    }
                    if !more {
                        break;
                    }
                }
                run_scan(&handle).await;
            }
        });
        Self { status: StdMutex::new(ScanStatus::default()), scan_lock: tokio::sync::Mutex::new(()), rescan: tx, watcher: StdMutex::new(None) }
    }

    pub fn request_scan(&self) {
        let _ = self.rescan.send(());
    }
}

/// Runs one scan now (skipped if one is already running) and tells the UI when it's done.
pub async fn run_scan(app: &AppHandle) -> Option<ScanReport> {
    let state = app.state::<AppState>();
    let Ok(_guard) = state.library.scan_lock.try_lock() else { return None };
    if scanner::roots(&state.db).await.map(|r| r.is_empty()).unwrap_or(true) {
        return None;
    }
    set_status(app, ScanStatus { running: true, phase: Some("walking".into()), done: 0, total: 0 });

    let client = match anilist::Client::new(auth::load_token().ok().flatten()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "scan skipped");
            return None;
        }
    };
    let progress_app = app.clone();
    let result = scanner::scan(&state.db, &client, move |p| {
        set_status(&progress_app, ScanStatus { running: true, phase: Some(p.phase.into()), done: p.done, total: p.total });
    })
    .await;
    set_status(app, ScanStatus::default());
    match result {
        Ok(report) => {
            let _ = app.emit(LIBRARY_CHANGED, ());
            Some(report)
        }
        Err(e) => {
            tracing::warn!(error = %e, "library scan failed");
            None
        }
    }
}

fn set_status(app: &AppHandle, status: ScanStatus) {
    let state = app.state::<AppState>();
    if let Ok(mut s) = state.library.status.lock() {
        *s = status.clone();
    }
    let _ = app.emit(SCAN_PROGRESS, status);
}

/// Watches every library folder; any video change triggers a (debounced) rescan.
pub async fn rewatch(app: &AppHandle) {
    let state = app.state::<AppState>();
    let roots = scanner::roots(&state.db).await.unwrap_or_default();
    let handle = app.clone();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if event.kind.is_access() {
            return;
        }
        if event.paths.iter().any(|p| scanner::is_video(p) || p.extension().is_none()) {
            handle.state::<AppState>().library.request_scan();
        }
    });
    let mut watcher = match watcher {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "can't watch library folders; changes are picked up at launch");
            return;
        }
    };
    for root in &roots {
        if let Err(e) = watcher.watch(std::path::Path::new(root), RecursiveMode::Recursive) {
            tracing::warn!(error = %e, root, "can't watch folder");
        }
    }
    if let Ok(mut slot) = state.library.watcher.lock() {
        *slot = Some(watcher);
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatus {
    roots: Vec<String>,
    scan: ScanStatus,
    last: Option<ScanReport>,
}

#[tauri::command]
pub async fn library_status(state: tauri::State<'_, AppState>) -> Result<LibraryStatus, String> {
    Ok(LibraryStatus {
        roots: scanner::roots(&state.db).await.map_err(msg)?,
        scan: state.library.status.lock().map(|s| s.clone()).unwrap_or_default(),
        last: scanner::last_scan(&state.db).await.map_err(msg)?,
    })
}

/// Opens the system folder picker over the main window. `None` when cancelled.
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let window = app.get_webview_window("main").ok_or("The main window isn't open.")?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Add a library folder").set_parent(&window).blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn add_library_root(app: AppHandle, state: tauri::State<'_, AppState>, path: String) -> Result<String, String> {
    let added = scanner::add_root(&state.db, &path).await.map_err(msg)?;
    rewatch(&app).await;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move { run_scan(&handle).await });
    Ok(added)
}

#[tauri::command]
pub async fn remove_library_root(app: AppHandle, state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    scanner::remove_root(&state.db, &path).await.map_err(msg)?;
    rewatch(&app).await;
    let _ = app.emit(LIBRARY_CHANGED, ());
    Ok(())
}

#[tauri::command]
pub async fn unmatched_files(state: tauri::State<'_, AppState>) -> Result<Vec<scanner::UnmatchedGroup>, String> {
    scanner::unmatched(&state.db).await.map_err(msg)
}

/// Matches files to a show by hand (from a search result). The show's details are fetched so
/// it has a title and art even when it isn't on the user's list.
#[tauri::command]
pub async fn assign_files(app: AppHandle, state: tauri::State<'_, AppState>, paths: Vec<String>, media_id: i64) -> Result<(), String> {
    let client = anilist::Client::new(auth::load_token().ok().flatten()).map_err(msg)?;
    media::page(&client, &state.db, media_id).await.map_err(msg)?;
    media::ensure_media_row(&state.db, media_id).await.map_err(msg)?;
    scanner::assign(&state.db, &paths, media_id).await.map_err(msg)?;
    let _ = app.emit(LIBRARY_CHANGED, ());
    Ok(())
}

#[tauri::command]
pub async fn ignore_files(app: AppHandle, state: tauri::State<'_, AppState>, paths: Vec<String>) -> Result<(), String> {
    scanner::ignore(&state.db, &paths).await.map_err(msg)?;
    let _ = app.emit(LIBRARY_CHANGED, ());
    Ok(())
}
