//! SQLite, tuned for a desktop app: WAL so reads never block the UI while a
//! scan writes, NORMAL sync (safe with WAL), memory-mapped reads, FK enforcement.

use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::Result;

pub type Db = sqlx::SqlitePool;

pub async fn open(path: &Path) -> Result<Db> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(30))
        .pragma("temp_store", "memory")
        .pragma("mmap_size", "268435456")
        .pragma("cache_size", "-16000");

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// In-memory database for tests.
pub async fn open_memory() -> Result<Db> {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
