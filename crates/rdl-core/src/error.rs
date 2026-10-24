use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("database migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("could not determine the user's application data folder")]
    NoDataDir,

    #[error("logging setup failed: {0}")]
    Logging(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
