//! B-Anime core: everything that isn't UI. The Tauri app (and a future
//! headless server) are thin shells over this crate.

pub mod db;
pub mod error;
pub mod home;
pub mod paths;

pub use error::{Error, Result};
pub use paths::AppPaths;
