//! Logging, with detailed network logs (iroh's connection and path decisions) switchable while
//! the app runs, and remembered. For troubleshooting Watch together connections, including on
//! phones, where RUST_LOG can't be set.
//!
//! Besides the terminal, logs go to `logs/b-anime.log` in the data folder (from the moment the
//! folder is known, at startup), so an installed copy's problems can be read afterwards. Past
//! 10 MB the file starts over, keeping the previous one as `b-anime.old.log`.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use banime_core::db;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Registry, fmt, reload};

use crate::{AppState, msg};

const KEY: &str = "debug.network_logs";
/// What the switch adds: iroh's connection, path and hole-punching decisions.
const DETAILED: &str = "info,iroh=debug,portmapper=debug,banime_core::watch=debug";

static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// Past this, the log file starts over (the previous one is kept as `b-anime.old.log`).
const FILE_LIMIT: u64 = 10 * 1024 * 1024;

struct LogFile {
    path: PathBuf,
    file: File,
    written: u64,
}

/// The log file, once the data folder is known (`to_file`).
static FILE: Mutex<Option<LogFile>> = Mutex::new(None);

/// Writes a log line to the file, when there is one.
struct FileWriter;

impl Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut slot) = FILE.lock() else { return Ok(buf.len()) };
        let Some(log) = slot.as_mut() else { return Ok(buf.len()) };
        if log.written + buf.len() as u64 > FILE_LIMIT {
            if let Ok(file) = start_over(&log.path) {
                log.file = file;
                log.written = 0;
            }
        }
        log.written += buf.len() as u64;
        log.file.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match FILE.lock().ok().as_mut().and_then(|s| s.as_mut()) {
            Some(log) => log.file.flush(),
            None => Ok(()),
        }
    }
}

/// Moves the current file aside (replacing the older one) and opens a fresh one.
fn start_over(path: &Path) -> io::Result<File> {
    let _ = std::fs::rename(path, path.with_file_name("b-anime.old.log"));
    OpenOptions::new().create(true).append(true).open(path)
}

/// RUST_LOG when set, otherwise "info"; the switch can raise it later.
pub fn init() {
    let base = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter, handle) = reload::Layer::new(base);
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(fmt::layer().with_ansi(false).with_writer(|| FileWriter))
        .init();
    let _ = FILTER.set(handle);
}

/// From now on, logs also go to `b-anime.log` in `dir` (appended; started over when too big).
pub fn to_file(dir: &Path) {
    let path = dir.join("b-anime.log");
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let opened = if size > FILE_LIMIT {
        start_over(&path).map(|f| (f, 0))
    } else {
        OpenOptions::new().create(true).append(true).open(&path).map(|f| (f, size))
    };
    match opened {
        Ok((file, written)) => {
            if let Ok(mut slot) = FILE.lock() {
                *slot = Some(LogFile { path, file, written });
            }
        }
        Err(e) => tracing::warn!(error = %e, path = %path.display(), "couldn't open the log file"),
    }
}

fn apply(on: bool) {
    // RUST_LOG, when set, wins over the switch (a developer asked for something specific).
    if std::env::var_os("RUST_LOG").is_some() {
        return;
    }
    if let Some(handle) = FILTER.get() {
        let _ = handle.reload(EnvFilter::new(if on { DETAILED } else { "info" }));
    }
}

async fn enabled(state: &AppState) -> bool {
    db::scalar_text(&state.db, "SELECT value FROM setting WHERE key = 'debug.network_logs'")
        .await
        .ok()
        .flatten()
        .is_some_and(|v| v == "1")
}

/// At launch: the remembered choice. Desktop only: the phone app has no such switch (and a
/// choice saved by a build that had one is ignored there).
pub async fn restore(state: &AppState) {
    if cfg!(mobile) {
        return;
    }
    if enabled(state).await {
        apply(true);
        tracing::info!("detailed network logs are on");
    }
}

#[tauri::command]
pub async fn network_logs(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(enabled(&state).await)
}

/// Shows the log file in Finder / Explorer (Settings → About).
#[tauri::command]
pub fn reveal_logs(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let file = state.paths.logs.join("b-anime.log");
    let target = if file.exists() { file } else { state.paths.logs.clone() };
    app.opener().reveal_item_in_dir(target).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_network_logs(state: tauri::State<'_, AppState>, on: bool) -> Result<(), String> {
    db::set_setting(&state.db, KEY, if on { "1" } else { "0" }).await.map_err(msg)?;
    apply(on);
    tracing::info!(on, "detailed network logs");
    Ok(())
}
