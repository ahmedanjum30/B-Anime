//! "What's new": the first launch after an update shows that version's changes, once. The notes
//! are CHANGELOG.md's section for the version: the release pipeline extracts just that section
//! (scripts/set-version.mjs) and hands it to the compiler as BANIME_RELEASE_NOTES, so only those
//! lines are built in (never the file) and they match what's installed, offline.

use banime_core::db;
use serde::Serialize;
use tauri::AppHandle;

use crate::{AppState, msg};

/// This version's notes, set by the release pipeline. Unset in local builds: no window.
const NOTES: Option<&str> = option_env!("BANIME_RELEASE_NOTES");

fn notes() -> Option<String> {
    NOTES.map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned)
}
/// The version whose notes were shown last (or that was installed fresh).
const SEEN_KEY: &str = "whats_new.seen";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    pub version: String,
    /// The section's Markdown, without its heading.
    pub notes: String,
}

/// A version's section of the changelog; a beta (x.y.z-beta.N) reads its version's. (The
/// release pipeline does the same in set-version.mjs; this serves the development preview.)
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn section(changelog: &str, version: &str) -> Option<String> {
    let base = version.split_once('-').map_or(version, |(b, _)| b);
    let heading = format!("## [{base}]");
    let mut lines = changelog.lines().skip_while(|l| !l.starts_with(&heading));
    lines.next()?;
    let notes: Vec<&str> = lines.take_while(|l| !l.starts_with("## [")).collect();
    let notes = notes.join("\n").trim().to_owned();
    (!notes.is_empty()).then_some(notes)
}

/// The newest released version in the changelog (for previewing in a development build).
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn latest(changelog: &str) -> Option<&str> {
    changelog.lines().find_map(|l| l.strip_prefix("## [")?.split_once(']').map(|(v, _)| v).filter(|v| *v != "Unreleased"))
}

async fn seen(state: &AppState) -> Option<String> {
    db::scalar_text(&state.db, "SELECT value FROM setting WHERE key = 'whats_new.seen'").await.ok().flatten()
}

/// Whether this database was in use before (an update, not a fresh install): any list, library
/// folder, extension or setting.
async fn used_before(state: &AppState) -> bool {
    db::scalar_text(
        &state.db,
        "SELECT 'y' WHERE EXISTS (SELECT 1 FROM list_entry) OR EXISTS (SELECT 1 FROM library_root)
           OR EXISTS (SELECT 1 FROM extension) OR EXISTS (SELECT 1 FROM setting WHERE key <> 'whats_new.seen')",
    )
    .await
    .ok()
    .flatten()
    .is_some()
}

/// This version's notes, when it's newly installed over an older one and they haven't been shown.
/// A fresh install shows nothing (and starts counting from here). Development builds never show it.
#[tauri::command]
pub async fn whats_new(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Option<WhatsNew>, String> {
    // Development builds: BANIME_WHATS_NEW=1 shows the newest section of the source folder's
    // CHANGELOG.md every launch, read at runtime (to see how it looks while working on it).
    #[cfg(debug_assertions)]
    if std::env::var("BANIME_WHATS_NEW").is_ok_and(|v| v == "1") {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md");
        let Ok(log) = std::fs::read_to_string(path) else { return Ok(None) };
        let Some(newest) = latest(&log) else { return Ok(None) };
        return Ok(section(&log, newest).map(|notes| WhatsNew { version: newest.to_owned(), notes }));
    }
    let version = app.package_info().version.to_string();
    let last = seen(&state).await;
    if last.as_deref() == Some(version.as_str()) {
        return Ok(None);
    }
    if cfg!(debug_assertions) || (last.is_none() && !used_before(&state).await) {
        db::set_setting(&state.db, SEEN_KEY, &version).await.map_err(msg)?;
        return Ok(None);
    }
    Ok(notes().map(|notes| WhatsNew { version, notes }))
}

/// The notes were shown: not again for this version.
#[tauri::command]
pub async fn whats_new_seen(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    db::set_setting(&state.db, SEEN_KEY, &app.package_info().version.to_string()).await.map_err(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Changelog\n\n## [Unreleased]\n\n## [0.4.5] - 2026-11-24\n\n### Added\n- **A.** One.\n  More.\n\n## [0.4.4] - 2026-11-24\n\n- Old.\n";

    #[test]
    fn reads_a_versions_section() {
        assert_eq!(section(LOG, "0.4.5").unwrap(), "### Added\n- **A.** One.\n  More.");
        assert_eq!(section(LOG, "0.4.5-beta.2").unwrap(), "### Added\n- **A.** One.\n  More.");
        assert_eq!(section(LOG, "0.4.4").unwrap(), "- Old.");
        assert!(section(LOG, "0.4.6").is_none());
    }

    #[test]
    fn the_newest_release_is_found() {
        assert_eq!(latest(LOG), Some("0.4.5"));
    }

    #[test]
    fn the_real_changelog_parses() {
        let log = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md")).unwrap();
        assert!(section(&log, "0.4.4").is_some());
    }
}
