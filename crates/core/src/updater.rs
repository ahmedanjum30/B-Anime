//! Self-update from GitHub Releases.
//!
//! A release carries the B-Installer setup (`B-Anime-Setup-<version>.exe`), the Android APK
//! (`B-Anime-<version>-arm64.apk`), the Mac app (`B-Anime-<version>-macos.zip`) and
//! `SHA256SUMS.txt` covering them. Each platform picks its own file. It's downloaded only from
//! GitHub's own hosts and its SHA-256 must match the checksum file. Windows then runs the setup
//! silently with `/UPDATE=1` (the installer closes B-Anime, replaces it, and starts it again);
//! Android hands the APK to the system installer (the app does that); the Mac swaps its app for
//! the zip's once it has quit, and opens it again.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub const REPO: &str = "ahmedanjum30/B-Anime";
/// Betas (`x.y.z-beta.N`) are published here, on their own Releases page, so B-Anime's page only
/// lists stable versions. Copies on the Beta channel look in both.
pub const BETA_REPO: &str = "ahmedanjum30/B-Anime-Beta";

/// A release version: `x.y.z`, or `x.y.z-beta.N` for the Nth beta of `x.y.z`. A beta comes before
/// its final release: 0.4.5-beta.1 < 0.4.5-beta.2 < 0.4.5 < 0.4.6-beta.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// `u32::MAX` for a final release, so it sorts after all its betas.
    pub beta: u32,
}

impl Version {
    pub fn parse(v: &str) -> Option<Version> {
        let v = v.trim().trim_start_matches('v');
        let (core, beta) = match v.split_once('-') {
            None => (v, u32::MAX),
            Some((core, pre)) => (core, pre.strip_prefix("beta.")?.parse().ok()?),
        };
        let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
        let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next()??);
        if parts.next().is_some() {
            return None;
        }
        Some(Version { major, minor, patch, beta })
    }

    pub fn is_beta(&self) -> bool {
        self.beta != u32::MAX
    }
}

/// Whether `candidate` is a later version than `current`. Unreadable versions are never newer.
pub fn newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        _ => false,
    }
}

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
    } else if cfg!(target_os = "macos") {
        format!("B-Anime-{version}-macos.zip")
    } else {
        format!("B-Anime-Setup-{version}.exe")
    }
}

async fn releases(http: &reqwest::Client, url: &str) -> Result<Vec<Release>> {
    let res = http
        .get(url)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    if res.status().as_u16() == 404 {
        return Ok(Vec::new()); // no releases (or no beta repository) yet
    }
    if !res.status().is_success() {
        return Err(Error::Network(format!("GitHub answered HTTP {}", res.status().as_u16())));
    }
    if url.ends_with("/latest") {
        let rel: Release = res.json().await.map_err(|e| Error::Network(e.to_string()))?;
        return Ok(vec![rel]);
    }
    res.json().await.map_err(|e| Error::Network(e.to_string()))
}

/// The newest release, when it's newer than `current`. Stable takes B-Anime's latest release;
/// `beta` also takes betas, from the beta repository (and any pre-release on B-Anime's page).
pub async fn check(current: &str, beta: bool) -> Result<Option<Update>> {
    let http = client()?;
    let mut list = releases(&http, &format!("https://api.github.com/repos/{REPO}/releases/latest")).await?;
    if beta {
        // An unreachable beta repository mustn't stop stable updates on the Beta channel.
        for url in [
            format!("https://api.github.com/repos/{BETA_REPO}/releases?per_page=20"),
            format!("https://api.github.com/repos/{REPO}/releases?per_page=20"),
        ] {
            match releases(&http, &url).await {
                Ok(more) => list.extend(more),
                Err(e) => tracing::warn!(error = %e, url, "beta releases unreachable"),
            }
        }
    }
    let Some(rel) = newest(list, beta) else { return Ok(None) };
    let version = rel.tag_name.trim_start_matches('v').to_owned();
    if !newer(&version, current) {
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

/// The highest version published (never a draft, never an unreadable tag); betas and
/// pre-releases only when `beta`.
fn newest(list: Vec<Release>, beta: bool) -> Option<Release> {
    list.into_iter()
        .filter_map(|r| Version::parse(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, r)| !r.draft && (beta || (!r.prerelease && !v.is_beta())))
        .max_by_key(|(v, _)| *v)
        .map(|(_, r)| r)
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

/// The version in an update file's name (the setup, the APK or the Mac app), if it's one of ours.
fn version_in(name: &str) -> Option<&str> {
    name.strip_prefix("B-Anime-Setup-")
        .and_then(|r| r.strip_suffix(".exe"))
        .or_else(|| name.strip_prefix("B-Anime-").and_then(|r| r.strip_suffix("-arm64.apk")))
        .or_else(|| name.strip_prefix("B-Anime-").and_then(|r| r.strip_suffix("-macos.zip")))
}

/// Deletes downloaded updates that are already installed (not newer than `current`), unfinished
/// downloads, and pending ones a newer download has replaced (a later beta, or a beta left behind
/// after switching back to Stable). The newest one not installed yet stays, so it isn't fetched
/// again. Returns how many files went.
pub fn clean_downloads(dir: &Path, current: &str) -> usize {
    clean(dir, |v| newer(v, current), None)
}

/// After `keep` has downloaded: every other download goes (it's the one to install now).
pub fn keep_only(dir: &Path, keep: &Path) -> usize {
    let keep = keep.file_name().map(|n| n.to_string_lossy().into_owned());
    clean(dir, |_| true, keep.as_deref())
}

fn clean(dir: &Path, pending: impl Fn(&str) -> bool, keep: Option<&str>) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let files: Vec<(String, std::path::PathBuf)> =
        entries.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
    // The one to keep: given, or the newest pending.
    let keep = keep.map(str::to_owned).or_else(|| {
        files
            .iter()
            .filter_map(|(n, _)| version_in(n).filter(|v| pending(v)).and_then(|v| Version::parse(v).map(|p| (p, n))))
            .max_by_key(|(p, _)| *p)
            .map(|(_, n)| n.clone())
    });
    let mut removed = 0;
    for (name, path) in &files {
        let ours = name.ends_with(".part") || version_in(name).is_some();
        if ours && keep.as_deref() != Some(name.as_str()) && std::fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Starts the verified setup silently in update mode. The caller should exit right after.
#[cfg(windows)]
pub fn launch_installer(setup: &Path) -> Result<()> {
    std::process::Command::new(setup)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/UPDATE=1"])
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Config(format!("couldn't start the installer: {e}")))
}

/// What replaces the Mac app: waits for B-Anime ($1, its process) to quit, unpacks the verified
/// zip ($2) and puts its B-Anime.app where the running one ($3) is, then opens it. The old app
/// stays aside until the new one is in place, and goes back if it can't be (a folder this user
/// can't write to, say): either way B-Anime opens again.
#[cfg(target_os = "macos")]
const SWAP_APP: &str = r#"
pid=$1; zip=$2; app=$3
n=0
while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 150 ]; do sleep 0.2; n=$((n + 1)); done
tmp=$(mktemp -d "${TMPDIR:-/tmp}/banime-update.XXXXXX") || { open "$app"; exit 1; }
if /usr/bin/ditto -x -k "$zip" "$tmp" && [ -d "$tmp/B-Anime.app" ]; then
  /usr/bin/xattr -dr com.apple.quarantine "$tmp/B-Anime.app" 2>/dev/null
  old="$app.replaced"
  rm -rf "$old"
  if mv "$app" "$old"; then
    if mv "$tmp/B-Anime.app" "$app"; then rm -rf "$old"; else mv "$old" "$app"; fi
  fi
fi
rm -rf "$tmp"
open "$app"
"#;

/// Swaps the running app for the verified zip's once this copy has quit, and opens it again.
/// The caller should exit right after.
#[cfg(target_os = "macos")]
pub fn launch_installer(zip: &Path) -> Result<()> {
    let exe = std::env::current_exe().and_then(|p| p.canonicalize())?;
    let app = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| Error::Config("B-Anime isn't running from its app, so it can't replace it".into()))?;
    use std::process::Stdio;
    std::process::Command::new("/bin/sh")
        .args(["-c", SWAP_APP, "banime-update"])
        .arg(std::process::id().to_string())
        .arg(zip)
        .arg(app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Config(format!("couldn't start the update: {e}")))
}

/// Android installs through the system installer (the app does that); nothing else is shipped.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn launch_installer(_setup: &Path) -> Result<()> {
    Err(Error::Config("this platform has no installer to run".into()))
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

    fn rel(tag: &str, prerelease: bool, draft: bool) -> Release {
        Release {
            tag_name: tag.into(),
            body: None,
            html_url: String::new(),
            published_at: None,
            draft,
            prerelease,
            assets: Vec::new(),
        }
    }

    #[test]
    fn betas_come_before_their_release() {
        assert!(newer("0.4.5-beta.2", "0.4.5-beta.1"));
        assert!(newer("0.4.5", "0.4.5-beta.9"));
        assert!(newer("0.4.5-beta.1", "0.4.4"));
        assert!(newer("0.4.6-beta.1", "0.4.5"));
        assert!(!newer("0.4.5-beta.3", "0.4.5"));
        assert!(!newer("0.4.5", "0.4.5"));
        assert!(newer("0.4.10", "0.4.9"));
        assert!(Version::parse("0.4.5-rc.1").is_none());
        assert!(Version::parse("0.4.5.1").is_none());
        assert!(!Version::parse("v0.4.5").unwrap().is_beta());
    }

    #[test]
    fn beta_takes_the_highest_prerelease_and_stable_skips_them() {
        let list = || vec![rel("v0.4.2", false, false), rel("v0.4.4", false, true), rel("v0.4.3", true, false)];
        assert_eq!(newest(list(), true).unwrap().tag_name, "v0.4.3");
        assert_eq!(newest(list(), false).unwrap().tag_name, "v0.4.2");
        // A stable release newer than the last beta wins on the beta channel too.
        let mut l = list();
        l.push(rel("v0.4.5", false, false));
        assert_eq!(newest(l, true).unwrap().tag_name, "v0.4.5");
        assert!(newest(vec![rel("v0.1.0", false, true)], true).is_none());
        // Numbered betas (published as plain releases in the beta repository).
        let l = || vec![rel("v0.4.4", false, false), rel("v0.4.5-beta.1", false, false), rel("v0.4.5-beta.2", false, false)];
        assert_eq!(newest(l(), true).unwrap().tag_name, "v0.4.5-beta.2");
        assert_eq!(newest(l(), false).unwrap().tag_name, "v0.4.4");
        let mut l = l();
        l.push(rel("v0.4.5", false, false));
        assert_eq!(newest(l, true).unwrap().tag_name, "v0.4.5");
    }

    /// Real network: `cargo test -p banime-core -- --ignored live_update`.
    #[tokio::test]
    #[ignore]
    async fn live_update_check_talks_to_github() {
        let result = check("0.0.1", false).await;
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
        let left = |dir: &Path| {
            let mut l: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
            l.sort();
            l
        };
        assert_eq!(left(&dir), ["B-Anime-0.2.9-arm64.apk", "notes.txt"]);

        // Two pending (an older beta never installed): only the newest stays.
        std::fs::write(dir.join("B-Anime-0.3.0-beta.1-arm64.apk"), b"x").unwrap();
        std::fs::write(dir.join("B-Anime-0.3.0-beta.2-arm64.apk"), b"x").unwrap();
        assert_eq!(clean_downloads(&dir, "0.2.8"), 2);
        assert_eq!(left(&dir), ["B-Anime-0.3.0-beta.2-arm64.apk", "notes.txt"]);

        // A fresh download replaces whatever was waiting.
        std::fs::write(dir.join("B-Anime-0.3.0-arm64.apk"), b"x").unwrap();
        assert_eq!(keep_only(&dir, &dir.join("B-Anime-0.3.0-arm64.apk")), 1);
        assert_eq!(left(&dir), ["B-Anime-0.3.0-arm64.apk", "notes.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_platform_installs_its_file() {
        let want = if cfg!(target_os = "android") {
            "B-Anime-0.3.0-arm64.apk"
        } else if cfg!(target_os = "macos") {
            "B-Anime-0.3.0-macos.zip"
        } else {
            "B-Anime-Setup-0.3.0.exe"
        };
        assert_eq!(asset_name("0.3.0"), want);
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
