use std::io;
use std::path::PathBuf;

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

    #[error("invalid stored setting: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("server returned HTTP {status} for {url}")]
    HttpStatus { status: u16, url: String },

    #[error("link expired or access denied (HTTP {0})")]
    LinkExpired(u16),

    #[error("the file on the server changed since the download started")]
    RemoteChanged,

    #[error("the server did not honour the byte-range request")]
    RangeNotHonored,

    #[error("too many redirects (last one to {0})")]
    TooManyRedirects(String),

    #[error("connection closed before the data was complete")]
    Incomplete,

    #[error("downloaded {actual} bytes but expected {expected}")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("destination already exists: {}", .0.display())]
    DestinationExists(PathBuf),

    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("{0} not found")]
    NotFound(String),

    #[error("not supported yet: {0}")]
    Unsupported(String),

    #[error("{0}")]
    Tool(String),

    #[error("checksum mismatch for {file}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    #[error("the network adapter for downloads ({0}) is down")]
    NetworkDown(String),

    #[error("Credential Manager: {0}")]
    Secrets(String),

    #[error("canceled")]
    Canceled,
}

impl Error {
    /// Whether retrying the same request might succeed (network blips, 5xx, 429).
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Http(e) => !e.is_builder() && !e.is_redirect(),
            Error::HttpStatus { status, .. } => matches!(status, 408 | 425 | 429 | 500..=599),
            Error::Incomplete => true,
            _ => false,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
