//! Watch together, app side: one room at a time, its events forwarded to the UI.

use banime_core::db;
use banime_core::watch::{self as room, Check, PlayState, Room, RoomEvent, RoomInfo};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::{AppState, msg};

const WATCH_STATE: &str = "watch-state";
const WATCH_ROSTER: &str = "watch-roster";
const WATCH_CLOSED: &str = "watch-closed";
const WATCH_INFO: &str = "watch-info";
const WATCH_LINK: &str = "watch-link";

#[derive(Default)]
pub struct Watch {
    room: Mutex<Option<Room>>,
}

fn forward(app: AppHandle, mut events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let _ = match event {
                RoomEvent::State(s) => app.emit(WATCH_STATE, &s),
                RoomEvent::Roster(r) => app.emit(WATCH_ROSTER, &r),
                RoomEvent::Closed(reason) => app.emit(WATCH_CLOSED, &reason),
                RoomEvent::Info(info) => app.emit(WATCH_INFO, &info),
                // "reconnecting" / "connected": the link to the room dropped, or is back.
                RoomEvent::Reconnecting => app.emit(WATCH_LINK, "reconnecting"),
                RoomEvent::Reconnected => app.emit(WATCH_LINK, "connected"),
            };
        }
    });
}

/// Replaces any room this device is in.
async fn enter(app: AppHandle, state: &AppState, made: banime_core::Result<(Room, tokio::sync::mpsc::UnboundedReceiver<RoomEvent>)>) -> Result<RoomInfo, String> {
    let (room, events) = made.map_err(msg)?;
    let info = room.info();
    let old = state.watch.room.lock().await.replace(room);
    if let Some(old) = old {
        old.leave(false).await;
    }
    forward(app, events);
    Ok(info)
}

#[tauri::command]
pub async fn watch_host(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
    room_name: Option<String>,
) -> Result<RoomInfo, String> {
    let seed = device_seed(&state).await?;
    enter(app, &state, Room::host(name, room_name, Some(seed)).await).await
}

/// This device's room code is made from a seed kept in its settings, so it hosts under the same
/// code (and invite link) every time, until New link replaces it.
const SEED_KEY: &str = "watch.seed";

async fn device_seed(state: &AppState) -> Result<u64, String> {
    let saved = db::scalar_text(&state.db, "SELECT value FROM setting WHERE key = 'watch.seed'").await.map_err(msg)?;
    if let Some(seed) = saved.and_then(|s| s.parse::<u64>().ok()) {
        return Ok(seed);
    }
    let seed = room::new_seed();
    db::set_setting(&state.db, SEED_KEY, &seed.to_string()).await.map_err(msg)?;
    Ok(seed)
}

/// The code this device hosts under (shown before a room is open).
#[tauri::command]
pub async fn watch_my_code(state: tauri::State<'_, AppState>) -> Result<String, String> {
    Ok(room::code_for_seed(device_seed(&state).await?))
}

/// A new code (and invite link) for this device; the old one stops working. For the next room.
#[tauri::command]
pub async fn watch_new_link(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let seed = room::new_seed();
    db::set_setting(&state.db, SEED_KEY, &seed.to_string()).await.map_err(msg)?;
    Ok(room::code_for_seed(seed))
}

/// Back into a room this device was in, through whichever of its members hosts it now.
#[tauri::command]
pub async fn watch_rejoin(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    codes: Vec<String>,
    name: String,
    so_far: Option<room::RoomSoFar>,
    hosted: Option<bool>,
) -> Result<RoomInfo, String> {
    let seed = device_seed(&state).await?;
    enter(app, &state, Room::rejoin(&codes, name, Some(seed), so_far, hosted.unwrap_or(false)).await).await
}

#[tauri::command]
pub async fn watch_join(app: AppHandle, state: tauri::State<'_, AppState>, code: String, name: String) -> Result<RoomInfo, String> {
    let seed = device_seed(&state).await?;
    enter(app, &state, Room::join(&code, name, Some(seed)).await).await
}

/// Leaves the room. A host either ends it for everyone (`end`) or hands it to the next guest.
#[tauri::command]
pub async fn watch_leave(state: tauri::State<'_, AppState>, end: Option<bool>) -> Result<(), String> {
    let room = state.watch.room.lock().await.take();
    if let Some(room) = room {
        room.leave(end.unwrap_or(false)).await;
    }
    Ok(())
}

/// The room this device is in, if any (the UI asks after a reload).
#[tauri::command]
pub async fn watch_room(state: tauri::State<'_, AppState>) -> Result<Option<RoomInfo>, String> {
    Ok(state.watch.room.lock().await.as_ref().map(Room::info))
}

#[tauri::command]
pub async fn watch_send_state(state: tauri::State<'_, AppState>, play: PlayState) -> Result<(), String> {
    if let Some(room) = state.watch.room.lock().await.as_ref() {
        room.send_state(play);
    }
    Ok(())
}

#[tauri::command]
pub async fn watch_send_status(
    state: tauri::State<'_, AppState>,
    position: f64,
    buffering: bool,
    media_id: Option<i64>,
    episode: Option<f64>,
    length: Option<f64>,
) -> Result<(), String> {
    if let Some(room) = state.watch.room.lock().await.as_ref() {
        room.send_status(position, buffering, media_id.zip(episode), length.filter(|l| l.is_finite() && *l > 0.0));
    }
    Ok(())
}

/// Ready to start (or not), in the lobby.
#[tauri::command]
pub async fn watch_ready(state: tauri::State<'_, AppState>, ready: bool) -> Result<(), String> {
    if let Some(room) = state.watch.room.lock().await.as_ref() {
        room.set_ready(ready);
    }
    Ok(())
}

/// The host starts the room: everyone plays the chosen episode.
#[tauri::command]
pub async fn watch_start(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let room = state.watch.room.lock().await;
    let room = room.as_ref().ok_or("You're not in a room.")?;
    if !room.is_host() {
        return Err("Only the host starts the room.".into());
    }
    if !room.start() {
        return Err("Pick an episode first.".into());
    }
    Ok(())
}

/// Whether this device can play the room's episode, and with what.
#[tauri::command]
pub async fn watch_check(state: tauri::State<'_, AppState>, playable: Option<bool>, source: Option<String>) -> Result<(), String> {
    if let Some(room) = state.watch.room.lock().await.as_ref() {
        room.set_check(Check { playable, source });
    }
    Ok(())
}

/// This device's name in the room.
#[tauri::command]
pub async fn watch_rename(state: tauri::State<'_, AppState>, name: String) -> Result<(), String> {
    if let Some(room) = state.watch.room.lock().await.as_ref() {
        room.set_name(&name);
    }
    Ok(())
}

/// The room's name (host only).
#[tauri::command]
pub async fn watch_set_room_name(state: tauri::State<'_, AppState>, name: String) -> Result<(), String> {
    let room = state.watch.room.lock().await;
    let room = room.as_ref().ok_or("You're not in a room.")?;
    if !room.set_room_name(&name) {
        return Err("Only the host names the room.".into());
    }
    Ok(())
}
