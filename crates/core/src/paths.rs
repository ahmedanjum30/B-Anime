//! Where B-Anime keeps user data.
//!
//! Default: `<OS config dir>/B-Anime` (`%APPDATA%\B-Anime` on Windows,
//! `~/Library/Application Support/B-Anime` on macOS, `~/.config/B-Anime` on Linux).
//! Override with the `B_ANIME_DATA_DIR` environment variable (absolute path).

use std::path::PathBuf;

use crate::{Error, Result};

pub const APP_DIR_NAME: &str = "B-Anime";
pub const DATA_DIR_ENV: &str = "B_ANIME_DATA_DIR";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub db: PathBuf,
    pub cache: PathBuf,
    pub transcode: PathBuf,
    pub logs: PathBuf,
    pub extensions: PathBuf,
    pub manga: PathBuf,
    pub offline: PathBuf,
}

impl AppPaths {
    /// Resolve the data directory: explicit override, then env var, then OS default.
    pub fn resolve(override_dir: Option<PathBuf>) -> Result<Self> {
        let chosen = override_dir.or_else(|| std::env::var_os(DATA_DIR_ENV).map(PathBuf::from));
        let root = match chosen {
            Some(dir) if !dir.is_absolute() => {
                return Err(Error::Config(format!(
                    "data directory must be an absolute path, got {}",
                    dir.display()
                )));
            }
            Some(dir) => dir,
            None => dirs::config_dir()
                .ok_or_else(|| Error::Config("could not locate the OS config directory".into()))?
                .join(APP_DIR_NAME),
        };
        Ok(Self::at(root))
    }

    pub fn at(root: PathBuf) -> Self {
        let cache = root.join("cache");
        Self {
            db: root.join("b-anime.db"),
            transcode: cache.join("transcode"),
            logs: root.join("logs"),
            extensions: root.join("extensions"),
            manga: root.join("manga"),
            offline: root.join("offline"),
            cache,
            root,
        }
    }

    /// Create every directory the app writes to.
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            &self.root,
            &self.cache,
            &self.transcode,
            &self.logs,
            &self.extensions,
            &self.manga,
            &self.offline,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_override_is_rejected() {
        assert!(AppPaths::resolve(Some(PathBuf::from("relative/dir"))).is_err());
    }

    #[test]
    fn layout_hangs_off_root() {
        let root = std::env::temp_dir().join("banime-test");
        let p = AppPaths::at(root.clone());
        assert_eq!(p.db, root.join("b-anime.db"));
        assert_eq!(p.transcode, root.join("cache").join("transcode"));
    }
}
