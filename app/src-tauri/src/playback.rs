//! Playing local files and external players (MPV with progress tracking, VLC without).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use banime_core::anilist::{self, auth, media};
use banime_core::{db, scanner, streaming};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, LIBRARY_CHANGED, msg};

/// Sent when an external player session marks an episode watched.
const PLAYER_WATCHED: &str = "player-watched";
const WATCHED_AT: f64 = 0.8;
const PLAYER_PREF: &str = "playback.player";

/// MPV or VLC, from PATH or their usual install folders.
pub fn find_player(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(&exe)).collect())
        .unwrap_or_default();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
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
pub fn external_players() -> Vec<String> {
    ["mpv", "vlc"].into_iter().filter(|p| find_player(p).is_some()).map(str::to_owned).collect()
}

/// "builtin", "mpv" or "vlc". Defaults to MPV when it's installed: it plays every format and
/// B-Anime can track progress in it.
async fn preferred_player(db: &db::Db) -> String {
    let saved = db::scalar_text(db, "SELECT value FROM setting WHERE key = 'playback.player'").await.ok().flatten();
    match saved.as_deref() {
        Some(p @ ("builtin" | "mpv" | "vlc")) if p == "builtin" || find_player(p).is_some() => p.to_owned(),
        _ if find_player("mpv").is_some() => "mpv".into(),
        _ => "builtin".into(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPrefs {
    pub player: String,
    pub available: Vec<String>,
}

#[tauri::command]
pub async fn get_player_prefs(state: tauri::State<'_, AppState>) -> Result<PlayerPrefs, String> {
    Ok(PlayerPrefs { player: preferred_player(&state.db).await, available: external_players() })
}

#[tauri::command]
pub async fn set_player_pref(state: tauri::State<'_, AppState>, player: String) -> Result<(), String> {
    if !matches!(player.as_str(), "builtin" | "mpv" | "vlc") {
        return Err("Unknown player.".into());
    }
    db::set_setting(&state.db, PLAYER_PREF, &player).await.map_err(msg)
}

/// Episode numbers the library has for a show.
#[tauri::command]
pub async fn local_episodes(state: tauri::State<'_, AppState>, media_id: i64) -> Result<Vec<i64>, String> {
    scanner::episodes(&state.db, media_id).await.map_err(msg)
}

/// A library episode as a stream the built-in player can load.
#[tauri::command]
pub async fn local_stream(state: tauri::State<'_, AppState>, media_id: i64, episode: i64) -> Result<streaming::Stream, String> {
    let path = scanner::episode_file(&state.db, media_id, episode)
        .await
        .map_err(msg)?
        .ok_or_else(|| format!("Episode {episode} isn't in your library."))?;
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err(format!("{} is missing. It may have been moved or deleted.", path.display()));
    }
    let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(streaming::Stream {
        sources: vec![streaming::StreamSource { url: state.proxy.file_url(&path), kind: "file".into(), label }],
        subtitles: vec![],
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayOutcome {
    /// "builtin": open the in-app player; "mpv" / "vlc": launched there.
    pub mode: String,
}

/// Plays a library episode with the preferred player.
#[tauri::command]
pub async fn play_local(app: AppHandle, state: tauri::State<'_, AppState>, media_id: i64, episode: i64) -> Result<PlayOutcome, String> {
    let player = preferred_player(&state.db).await;
    if player == "builtin" {
        return Ok(PlayOutcome { mode: player });
    }
    let path = scanner::episode_file(&state.db, media_id, episode)
        .await
        .map_err(msg)?
        .ok_or_else(|| format!("Episode {episode} isn't in your library."))?;
    if !std::path::Path::new(&path).is_file() {
        return Err(format!("{path} is missing. It may have been moved or deleted."));
    }
    let title = show_title(&state.db, media_id).await;
    let start = streaming::position(&state.db, media_id, episode).await.ok().flatten().filter(|s| *s > 30);
    launch(&app, &player, &path, &format!("{title} · Episode {episode}"), None, start, Some((media_id, episode)))?;
    Ok(PlayOutcome { mode: player })
}

async fn show_title(db: &db::Db, media_id: i64) -> String {
    media::title_and_total(db, media_id).await.ok().and_then(|(t, _)| t).unwrap_or_default()
}

/// Opens a proxied stream in MPV or VLC (from the built-in player's menu).
#[tauri::command]
pub fn open_external_player(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    player: String,
    url: String,
    title: String,
    subtitle: Option<String>,
    media_id: Option<i64>,
    episode: Option<i64>,
) -> Result<(), String> {
    let origin = state.proxy.origin();
    if !url.starts_with(&origin) || subtitle.as_deref().is_some_and(|s| !s.starts_with(&origin)) {
        return Err("Only B-Anime streams can be opened externally.".into());
    }
    launch(&app, &player, &url, &title, subtitle.as_deref(), None, media_id.zip(episode))
}

fn launch(
    app: &AppHandle,
    player: &str,
    target: &str,
    title: &str,
    subtitle: Option<&str>,
    start: Option<i64>,
    track: Option<(i64, i64)>,
) -> Result<(), String> {
    let exe = find_player(player).ok_or_else(|| format!("{player} isn't installed."))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(target);
    match player {
        "mpv" => {
            cmd.arg(format!("--force-media-title={title}"));
            if let Some(s) = subtitle {
                cmd.arg(format!("--sub-file={s}"));
            }
            if let Some(s) = start {
                cmd.arg(format!("--start={s}"));
            }
            let endpoint = track.map(|_| banime_core::player::ipc_endpoint());
            if let Some(ep) = &endpoint {
                cmd.arg(format!("--input-ipc-server={ep}"));
            }
            cmd.spawn().map_err(|e| format!("Couldn't start MPV: {e}"))?;
            if let (Some(endpoint), Some((media_id, episode))) = (endpoint, track) {
                spawn_tracker(app.clone(), endpoint, media_id, episode);
            }
        }
        _ => {
            cmd.arg(format!("--meta-title={title}"));
            if let Some(s) = subtitle {
                cmd.arg(format!("--input-slave={s}"));
            }
            if let Some(s) = start {
                cmd.arg(format!("--start-time={s}"));
            }
            cmd.spawn().map_err(|e| format!("Couldn't start VLC: {e}"))?;
        }
    }
    Ok(())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Watched {
    media_id: i64,
    episode: i64,
    title: String,
}

/// Follows an MPV session: saves the position every few seconds and records the episode as
/// watched on AniList once 80% has played.
fn spawn_tracker(app: AppHandle, endpoint: String, media_id: i64, episode: i64) {
    tauri::async_runtime::spawn(async move {
        let (tx, mut rx) = tokio::sync::watch::channel((0.0_f64, 0.0_f64));
        let mut follow = Box::pin(banime_core::player::watch_mpv(&endpoint, move |p, d| {
            let _ = tx.send((p, d));
        }));
        let mut marked = false;
        let mut last_save = Instant::now();
        loop {
            tokio::select! {
                done = &mut follow => {
                    if let Err(e) = done {
                        tracing::warn!(error = %e, "MPV tracking ended");
                    }
                    break;
                }
                changed = rx.changed() => {
                    if changed.is_err() { break; }
                    let (pos, dur) = *rx.borrow();
                    let state = app.state::<AppState>();
                    if dur > 0.0 && last_save.elapsed() > Duration::from_secs(5) {
                        last_save = Instant::now();
                        let _ = streaming::save_position(&state.db, media_id, episode, pos, dur).await;
                    }
                    if !marked && dur > 0.0 && pos / dur >= WATCHED_AT {
                        marked = true;
                        if let Err(e) = mark_watched(&app, &state, media_id, episode).await {
                            tracing::warn!(error = %e, "couldn't record MPV progress on AniList");
                        }
                    }
                }
            }
        }
        // Final position, so the next play resumes there.
        let (pos, dur) = *rx.borrow();
        if dur > 0.0 {
            let state = app.state::<AppState>();
            let _ = streaming::save_position(&state.db, media_id, episode, pos, dur).await;
        }
    });
}

async fn mark_watched(app: &AppHandle, state: &AppState, media_id: i64, episode: i64) -> Result<(), String> {
    let entry = media::local_entry(&state.db, media_id).await.map_err(msg)?;
    let total = media::title_and_total(&state.db, media_id).await.ok().and_then(|(_, t)| t);
    let Some(change) = media::progress_change(entry.as_ref(), episode, total) else { return Ok(()) };
    let token = auth::load_token().map_err(msg)?.ok_or("Not signed in to AniList.")?;
    let client = anilist::Client::new(Some(token)).map_err(msg)?;
    media::save_entry(&client, &state.db, media_id, change).await.map_err(msg)?;
    let title = show_title(&state.db, media_id).await;
    let _ = app.emit(PLAYER_WATCHED, Watched { media_id, episode, title });
    let _ = app.emit(LIBRARY_CHANGED, ());
    Ok(())
}
