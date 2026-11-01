//! First contact with a URL: final location, size, range support, validators
//! and a filename. Uses `GET` with `Range: bytes=0-0` rather than `HEAD`,
//! because plenty of servers answer `HEAD` differently (or not at all).

use reqwest::header::{
    ACCEPT_ENCODING, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG,
    HeaderMap, HeaderValue, LAST_MODIFIED, RANGE,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use reqwest::{Client, StatusCode, Url};

use super::{check_status, filename, parse_content_range};
use crate::Result;
use crate::accounts::Login;
use crate::request::RequestContext;

/// What to fetch, and what the browser would send with it (referer, user
/// agent, cookies), plus a site login to answer HTTP Basic challenges.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub url: Url,
    pub request: RequestContext,
    /// Given only to its own site, and only once that site asked for it.
    pub login: Option<Login>,
    /// Hosts that asked for Basic auth; later requests to them send the
    /// login straight away rather than being refused first.
    pub(super) basic_hosts: Arc<Mutex<HashSet<String>>>,
}

impl DownloadSpec {
    pub fn new(url: Url) -> Self {
        Self::with_request(url, RequestContext::default())
    }

    pub fn with_request(url: Url, request: RequestContext) -> Self {
        Self {
            url,
            request,
            login: None,
            basic_hosts: Arc::default(),
        }
    }

    pub fn with_login(mut self, login: Option<Login>) -> Self {
        self.login = login;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// URL after redirects; segment requests go here directly.
    pub final_url: Url,
    pub size: Option<u64>,
    pub accepts_ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    /// Already sanitised for Windows.
    pub filename: String,
}

impl ProbeResult {
    /// Value for `If-Range`. Only strong ETags are allowed there, so a weak one
    /// falls back to `Last-Modified`.
    pub fn validator(&self) -> Option<&str> {
        self.etag
            .as_deref()
            .filter(|e| !e.starts_with("W/"))
            .or(self.last_modified.as_deref())
    }

    /// Segmented download needs both range support and a known size.
    pub fn is_segmentable(&self) -> bool {
        self.accepts_ranges && self.size.is_some_and(|s| s > 0)
    }
}

pub async fn probe(client: &Client, spec: &DownloadSpec) -> Result<ProbeResult> {
    let mut extra = HeaderMap::new();
    extra.insert(RANGE, HeaderValue::from_static("bytes=0-0"));
    extra.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    let response = super::get(client, spec, &spec.url, &extra).await?;
    let status = response.status();
    let final_url = response.url().clone();
    check_status(status, &final_url)?;

    let headers = response.headers();
    let header = |name| {
        headers.get(name).map(|v: &reqwest::header::HeaderValue| {
            String::from_utf8_lossy(v.as_bytes()).into_owned()
        })
    };

    let (size, accepts_ranges) = if status == StatusCode::PARTIAL_CONTENT {
        let total = header(CONTENT_RANGE)
            .as_deref()
            .and_then(parse_content_range)
            .and_then(|(_, total)| total);
        (total, true)
    } else {
        // The server ignored the range and is sending the whole body.
        let length = header(CONTENT_LENGTH).and_then(|v| v.trim().parse().ok());
        (length, false)
    };

    let name = header(CONTENT_DISPOSITION)
        .as_deref()
        .and_then(filename::from_content_disposition)
        .or_else(|| filename::from_url(&final_url))
        .unwrap_or_else(|| filename::FALLBACK_NAME.to_owned());

    let result = ProbeResult {
        size,
        accepts_ranges,
        etag: header(ETAG),
        last_modified: header(LAST_MODIFIED),
        content_type: header(CONTENT_TYPE),
        filename: filename::sanitize(&name),
        final_url,
    };
    tracing::debug!(url = %spec.url, ?result, "probed");
    // Dropping the response here closes the connection without reading the body.
    Ok(result)
}
