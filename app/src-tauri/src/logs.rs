//! Logging, with detailed network logs (iroh's connection and path decisions) switchable while
//! the app runs, and remembered. For troubleshooting Watch together connections, including on
//! phones, where RUST_LOG can't be set.

use std::sync::OnceLock;

use banime_core::db;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Registry, fmt, reload};

use crate::{AppState, msg};

const KEY: &str = "debug.network_logs";
/// What the switch adds: iroh's connection, path and hole-punching decisions.
const DETAILED: &str = "info,iroh=debug,portmapper=debug,banime_core::watch=debug";

static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// RUST_LOG when set, otherwise "info"; the switch can raise it later.
pub fn init() {
    let base = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter, handle) = reload::Layer::new(base);
    tracing_subscriber::registry().with(filter).with(fmt::layer()).init();
    let _ = FILTER.set(handle);
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

/// At launch: the remembered choice.
pub async fn restore(state: &AppState) {
    if enabled(state).await {
        apply(true);
        tracing::info!("detailed network logs are on");
    }
}

#[tauri::command]
pub async fn network_logs(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(enabled(&state).await)
}

#[tauri::command]
pub async fn set_network_logs(state: tauri::State<'_, AppState>, on: bool) -> Result<(), String> {
    db::set_setting(&state.db, KEY, if on { "1" } else { "0" }).await.map_err(msg)?;
    apply(on);
    tracing::info!(on, "detailed network logs");
    Ok(())
}
