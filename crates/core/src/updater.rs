//! Self-update from GitHub Releases.
//!
//! A release carries the B-Installer setup (`B-Anime-Setup-<version>.exe`), the Android APK
//! (`B-Anime-<version>-arm64.apk`) and `SHA256SUMS.txt` covering both. Each platform picks its own
//! file. It's downloaded only from GitHub's own hosts and its SHA-256 must match the checksum file.
//! Windows then runs the setup silently with `/UPDATE=1` (the installer closes B-Anime, replaces
//! it, and starts it again); Android hands the APK to the system installer (the app does that).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub const REPO: &str = "ahmedanjum30/B-Anime";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub version: String,
    pub notes: String,
    pub published_at: Option<String>,
    pub page_url: String,
    #[serde(skip)]
    pub setup_url: String,
    #[serde(skip)]
    pub setup_name: String,
    #[serde(skip)]
    pub sha256: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn client() -> Result<reqwest::Client> {
    crate::http::client_builder()
        .user_agent(concat!("B-Anime/", env!("CARGO_PKG_VERSION"), " (updater)"))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Network(e.to_string()))
}

/// Downloads come only from GitHub (release assets redirect to its CDN).
fn trusted(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some("github.com" | "objects.githubusercontent.com" | "release-assets.githubusercontent.com")
        )
}

/// The release file this platform installs from.
pub fn asset_name(version: &str) -> String {
    if cfg!(target_os = "android") {
        format!("B-Anime-{version}-arm64.apk")
    } else {
        format!("B-Anime-Setup-{version}.exe")
    }
}

/// The latest release, when it's newer than `current`.
pub async fn check(current: &str) -> Result<Option<Update>> {
    let http = client()?;
    let res = http
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    if res.status().as_u16() == 404 {
        return Ok(None); // no releases yet
    }
    if !res.status().is_success() {
        return Err(Error::Network(format!("GitHub answered HTTP {}", res.status().as_u16())));
    }
    let rel: Release = res.json().await.map_err(|e| Error::Network(e.to_string()))?;
    let version = rel.tag_name.trim_start_matches('v').to_owned();
    if rel.draft || rel.prerelease || !crate::extensions::newer(&version, current) {
        return Ok(None);
    }

    let setup_name = asset_name(&version);
    let setup = rel
        .assets
        .iter()
        .find(|a| a.name == setup_name)
        .ok_or_else(|| Error::Config(format!("release {version} has no {setup_name}")))?;
    let sums = rel
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS.txt")
        .ok_or_else(|| Error::Config(format!("release {version} has no checksums; not updating")))?;

    let sums_url = reqwest::Url::parse(&sums.browser_download_url).map_err(|e| Error::Config(e.to_string()))?;
    if !trusted(&sums_url) {
        return Err(Error::Config("checksums aren't hosted on GitHub; not updating".into()));
    }
    let text = http.get(sums_url).send().await.and_then(|r| r.error_for_status()).map_err(|e| Error::Network(e.to_string()))?;
    let text = text.text().await.map_err(|e| Error::Network(e.to_string()))?;
    let sha256 = text
        .lines()
        .find_map(|l| {
            let mut parts = l.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (name == setup_name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| Error::Config(format!("no checksum for {setup_name}; not updating")))?;

    Ok(Some(Update {
        version,
        notes: rel.body.unwrap_or_default(),
        published_at: rel.published_at,
        page_url: rel.html_url,
        setup_url: setup.browser_download_url.clone(),
        setup_name,
        sha256,
    }))
}

/// Downloads the setup into `dir` and verifies its checksum. Reuses an earlier verified download.
pub async fn download(update: &Update, dir: &Path) -> Result<PathBuf> {
    tokio::fs::create_dir_all(dir).await?;
    let target = dir.join(&update.setup_name);
    if let Ok(bytes) = tokio::fs::read(&target).await {
        if hex(&Sha256::digest(&bytes)) == update.sha256 {
            return Ok(target);
        }
    }

    let url = reqwest::Url::parse(&update.setup_url).map_err(|e| Error::Config(e.to_string()))?;
    if !trusted(&url) {
        return Err(Error::Config("the update isn't hosted on GitHub; not downloading".into()));
    }
    let http = crate::http::client_builder()
        .user_agent(concat!("B-Anime/", env!("CARGO_PKG_VERSION"), " (updater)"))
        .connect_timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() > 5 || !trusted(a.url()) { a.stop() } else { a.follow() }
        }))
        .build()
        .map_err(|e| Error::Network(e.to_string()))?;
    let mut res = http.get(url).send().await.and_then(|r| r.error_for_status()).map_err(|e| Error::Network(e.to_string()))?;

    let part = dir.join(format!("{}.part", update.setup_name));
    let mut file = tokio::fs::File::create(&part).await?;
    let mut hasher = Sha256::new();
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = res.chunk().await.map_err(|e| Error::Network(e.to_string()))? {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);

    if hex(&hasher.finalize()) != update.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(Error::Config("the downloaded update doesn't match its checksum; discarded".into()));
    }
    tokio::fs::rename(&part, &target).await?;
    Ok(target)
}

/// The version in an update file's name (the setup or the APK), if it's one of ours.
fn version_in(name: &str) -> Option<&str> {
    name.strip_prefix("B-Anime-Setup-")
        .and_then(|r| r.strip_suffix(".exe"))
        .or_else(|| name.strip_prefix("B-Anime-").and_then(|r| r.strip_suffix("-arm64.apk")))
}

/// Deletes downloaded updates that are already installed (not newer than `current`) and
/// unfinished downloads. A newer one that's downloaded but not yet installed stays, so it isn't
/// fetched again. Returns how many files went.
pub fn clean_downloads(dir: &Path, current: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stale = name.ends_with(".part") || version_in(&name).is_some_and(|v| !crate::extensions::newer(v, current));
        if stale && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Starts the verified setup silently in update mode. The caller should exit right after.
pub fn launch_installer(setup: &Path) -> Result<()> {
    std::process::Command::new(setup)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/UPDATE=1"])
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Config(format!("couldn't start the installer: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_github_hosts_are_trusted() {
        let ok = |u: &str| trusted(&reqwest::Url::parse(u).unwrap());
        assert!(ok("https://github.com/a/b/releases/download/v1/x.exe"));
        assert!(ok("https://objects.githubusercontent.com/x"));
        assert!(!ok("http://github.com/x"));
        assert!(!ok("https://github.com.evil.example/x"));
        assert!(!ok("https://example.com/x"));
    }

    /// Real network: `cargo test -p banime-core -- --ignored live_update`.
    #[tokio::test]
    #[ignore]
    async fn live_update_check_talks_to_github() {
        let result = check("0.0.1").await;
        println!("update check: {result:?}");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn installed_updates_are_cleaned_up() {
        let dir = std::env::temp_dir().join(format!("banime-updates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "B-Anime-0.2.7-arm64.apk",      // older: installed long ago
            "B-Anime-0.2.8-arm64.apk",      // the version now running
            "B-Anime-Setup-0.2.8.exe",      // the version now running
            "B-Anime-0.2.9-arm64.apk.part", // an unfinished download
            "B-Anime-0.2.9-arm64.apk",      // newer, not installed yet: keep
            "notes.txt",                    // not ours: keep
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        assert_eq!(clean_downloads(&dir, "0.2.8"), 4);
        let mut left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["B-Anime-0.2.9-arm64.apk", "notes.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn desktop_installs_the_setup() {
        assert_eq!(asset_name("0.3.0"), "B-Anime-Setup-0.3.0.exe");
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
