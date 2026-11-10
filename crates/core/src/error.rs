use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("database migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
