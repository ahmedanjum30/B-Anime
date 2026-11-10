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
    #[error("couldn't reach AniList: {0}")]
    Network(String),
    #[error("AniList error: {0}")]
    AniList(String),
    #[error("your AniList sign-in has expired; connect your account again")]
    Unauthorized,
    #[error("{0}")]
    Login(String),
    #[error("couldn't access the system credential store: {0}")]
    Credentials(String),
    #[error("extension error: {0}")]
    Extension(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
