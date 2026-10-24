//! Tracing setup: daily-rolling log files, plus stderr in debug builds.
//!
//! The filter comes from `PYDL_LOG` (same syntax as `RUST_LOG`), defaulting to `info`.

use std::path::Path;

pub use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::{Error, Result};

const DEFAULT_FILTER: &str = "info,sqlx=warn";
const MAX_LOG_FILES: usize = 14;

/// Install the global subscriber. Keep the returned guard alive for the life of
/// the process, or buffered log lines are lost on exit.
pub fn init(logs_dir: &Path) -> Result<WorkerGuard> {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("win-pydl")
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(logs_dir)
        .map_err(|e| Error::Logging(e.to_string()))?;
    let (file_writer, guard) = tracing_appender::non_blocking(appender);

    let filter =
        EnvFilter::try_from_env("PYDL_LOG").unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let stderr = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(file_writer).with_ansi(false))
        .with(stderr)
        .try_init()
        .map_err(|e| Error::Logging(e.to_string()))?;

    Ok(guard)
}
