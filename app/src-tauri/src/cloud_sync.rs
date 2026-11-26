//! Your place in each show across your devices, through your Google account (see
//! `banime_core::cloud`): sign in and out, and when it syncs. At launch, every 10 minutes, and
//! shortly after you stop watching (a burst of saves while watching makes one sync).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use banime_core::{cloud, db, Error};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, Notify};

use crate::{msg, AppState};

const EMAIL_KEY: &str = "google.email";
const SYNCED_KEY: &str = "google.synced_at";
/// Sent when places came from another device (the pages showing positions refresh).
const PLACES_CHANGED: &str = "places-changed";
const EVERY: Duration = Duration::from_secs(10 * 60);
/// After the last save while watching: one sync for the burst.
const AFTER_SAVE: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct CloudSync {
    session: cloud::Session,
    /// One sync at a time.
    running: Mutex<()>,
    /// A sync is already waiting to follow the latest save.
    queued: AtomicBool,
    error: std::sync::Mutex<Option<String>>,
    /// Wakes a sign-in that's waiting on the browser, to cancel it.
    cancel: Arc<Notify>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAccount {
    /// This build can sign in to Google (it has B-Anime's Google client).
    configured: bool,
    signed_in: bool,
    email: Option<String>,
    synced_at: Option<String>,
    error: Option<String>,
}

async fn account(state: &AppState) -> GoogleAccount {
    let signed_in = cloud::signed_in();
    GoogleAccount {
        configured: cloud::client().is_some(),
        signed_in,
        email: if signed_in {
            db::scalar_text(&state.db, "SELECT value FROM setting WHERE key = 'google.email'").await.ok().flatten()
        } else {
            None
        },
        synced_at: if signed_in {
            db::scalar_text(&state.db, "SELECT value FROM setting WHERE key = 'google.synced_at'").await.ok().flatten()
        } else {
            None
        },
        error: state.cloud.error.lock().ok().and_then(|e| e.clone()),
    }
}

fn set_error(state: &AppState, e: Option<String>) {
    if let Ok(mut slot) = state.cloud.error.lock() {
        *slot = e;
    }
}

/// One sync, if signed in. Errors are kept for Settings (and logged without any token).
async fn sync_now(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !cloud::signed_in() || cloud::client().is_none() {
        return;
    }
    let _one = state.cloud.running.lock().await;
    match cloud::sync(&state.db, &state.cloud.session).await {
        Ok(came) => {
            set_error(&state, None);
            let _ = db::set_setting(&state.db, SYNCED_KEY, &chrono::Utc::now().to_rfc3339()).await;
            if came {
                let _ = app.emit(PLACES_CHANGED, ());
            }
        }
        Err(Error::Unauthorized) => {
            // Signed out elsewhere, or access removed in Google's settings: signed out here too.
            let _ = db::set_setting(&state.db, EMAIL_KEY, "").await;
            set_error(&state, Some("Google signed B-Anime out. Sign in again to keep syncing.".into()));
        }
        Err(e) => {
            tracing::warn!(error = %e, "place sync failed");
            // The generic network error names AniList; this one is Google.
            let text = match e {
                Error::Network(m) => format!("Couldn't reach Google: {m}"),
                e => e.to_string(),
            };
            set_error(&state, Some(text));
        }
    }
}

/// At launch (once things have settled), then every 10 minutes.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            sync_now(&app).await;
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// A position was saved: one sync shortly after the last of a burst.
pub fn after_save(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !cloud::signed_in() || state.cloud.queued.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(AFTER_SAVE).await;
        app.state::<AppState>().cloud.queued.store(false, Ordering::SeqCst);
        sync_now(&app).await;
    });
}

#[tauri::command]
pub async fn google_account(state: tauri::State<'_, AppState>) -> Result<GoogleAccount, String> {
    Ok(account(&state).await)
}

/// Opens Google's sign-in in the browser and waits (up to 5 minutes) for it.
#[tauri::command]
pub async fn google_sign_in(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<GoogleAccount, String> {
    state.cloud.cancel.notify_waiters();
    let pending = cloud::start_sign_in().await.map_err(msg)?;
    app.opener().open_url(&pending.url, None::<&str>).map_err(|e| format!("Couldn't open your browser: {e}"))?;
    let cancel = state.cloud.cancel.clone();
    let email = tokio::select! {
        r = pending.wait(&state.cloud.session) => r.map_err(msg)?,
        _ = cancel.notified() => return Err("Sign-in cancelled.".into()),
    };
    db::set_setting(&state.db, EMAIL_KEY, email.as_deref().unwrap_or_default()).await.map_err(msg)?;
    set_error(&state, None);
    sync_now(&app).await;
    Ok(account(&state).await)
}

#[tauri::command]
pub async fn google_cancel_sign_in(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cloud.cancel.notify_waiters();
    Ok(())
}

/// Signs this device out (and Google forgets the sign-in). Places already synced stay in your
/// account; this device's own positions stay here.
#[tauri::command]
pub async fn google_sign_out(state: tauri::State<'_, AppState>) -> Result<GoogleAccount, String> {
    cloud::sign_out(&state.cloud.session).await.map_err(msg)?;
    db::set_setting(&state.db, EMAIL_KEY, "").await.map_err(msg)?;
    set_error(&state, None);
    Ok(account(&state).await)
}

#[tauri::command]
pub async fn google_sync(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<GoogleAccount, String> {
    sync_now(&app).await;
    Ok(account(&state).await)
}
