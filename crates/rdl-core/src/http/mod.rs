//! HTTP(S) downloading: probing, filenames, and the segmented engine.

mod download;
pub mod filename;
mod probe;
pub mod segments;

use std::time::Duration;

use reqwest::{StatusCode, Url, redirect};

use crate::{Error, Result};

pub use download::{Outcome, Progress, SegmentConfig, download};
pub use probe::{DownloadSpec, ProbeResult, probe};
pub use segments::Segment;

pub const DEFAULT_USER_AGENT: &str = concat!("RustyDownloader/", env!("CARGO_PKG_VERSION"));

/// Shared client for all HTTP downloads. Responses are never decompressed: byte
/// ranges must refer to the bytes on the wire.
pub fn client(user_agent: Option<&str>) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(user_agent.unwrap_or(DEFAULT_USER_AGENT))
        .redirect(redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .build()?)
}

/// Map a non-success status to the matching error.
pub(crate) fn check_status(status: StatusCode, url: &Url) -> Result<()> {
    match status.as_u16() {
        200..=299 => Ok(()),
        code @ (401 | 403 | 410) => Err(Error::LinkExpired(code)),
        code => Err(Error::HttpStatus {
            status: code,
            url: url.to_string(),
        }),
    }
}

/// Parse `bytes <start>-<end>/<total>`, returning `(start, total)`; total is
/// `None` when the server sends `*`.
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (start, _end) = range.split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    Some((start.trim().parse().ok()?, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parsing() {
        assert_eq!(parse_content_range("bytes 0-0/1234"), Some((0, Some(1234))));
        assert_eq!(parse_content_range("bytes 500-999/*"), Some((500, None)));
        assert_eq!(parse_content_range("bytes */1234"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
    }

    #[test]
    fn status_mapping() {
        let url = Url::parse("https://example.com/f").unwrap();
        assert!(check_status(StatusCode::PARTIAL_CONTENT, &url).is_ok());
        assert!(matches!(
            check_status(StatusCode::GONE, &url),
            Err(Error::LinkExpired(410))
        ));
        let err = check_status(StatusCode::SERVICE_UNAVAILABLE, &url).unwrap_err();
        assert!(err.is_retryable());
        let err = check_status(StatusCode::NOT_FOUND, &url).unwrap_err();
        assert!(!err.is_retryable());
    }
}
