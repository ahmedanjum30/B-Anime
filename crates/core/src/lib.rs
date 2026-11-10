//! B-Anime core: everything that isn't UI. The Tauri app (and a future
//! headless server) are thin shells over this crate.

pub mod anilist;
pub mod db;
pub mod error;
pub mod extensions;
pub mod home;
pub mod library;
pub mod mkv;
pub mod paths;
pub mod pgs;
pub mod player;
pub mod scanner;
pub mod subs;
pub mod updater;
pub mod streaming;

pub use error::{Error, Result};
pub use paths::AppPaths;
