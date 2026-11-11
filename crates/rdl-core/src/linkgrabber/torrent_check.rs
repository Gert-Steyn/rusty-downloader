//! The torrent side of link checking. A magnet's file list comes from the
//! engine (its metadata copy, or the peers within a timeout). A `.torrent`
//! file is read, or fetched over HTTP, and inspected locally. A check that
//! can't get the file list still leaves the link online (so it can be
//! confirmed), with a note saying why.

use std::time::Duration;

use url::Url;

use super::Inner;
use crate::http::ProbeResult;
use crate::http::filename;
use crate::proxy::ProxyChoice;
use crate::request::RequestContext;
use crate::torrent::{MAX_TORRENT_FILE, TorrentInfo, TorrentSource, fetch_http};
use crate::{Error, Result};

/// How long a magnet's metadata may take to arrive from the peers.
const MAGNET_TIMEOUT: Duration = Duration::from_secs(90);

/// What a check learned about a torrent: its file list, or why there is none.
#[derive(Debug, Default)]
pub(super) struct Checked {
    pub info: Option<TorrentInfo>,
    /// Why there is no file list; shown on the link.
    pub note: Option<String>,
}

impl Checked {
    fn found(info: TorrentInfo) -> Self {
        Self {
            info: Some(info),
            note: None,
        }
    }

    fn missing(note: String) -> Self {
        Self {
            info: None,
            note: Some(note),
        }
    }
}

/// The file list of a magnet, from the engine. Without an engine there is
/// none, and the link is checked as before (name and size from the link).
pub(super) async fn check_magnet(inner: &Inner, url: &Url) -> Checked {
    let Some(engine) = &inner.torrents else {
        return Checked::default();
    };
    let source = TorrentSource::Magnet(url.as_str().to_owned());
    match tokio::time::timeout(MAGNET_TIMEOUT, engine.inspect(source)).await {
        Ok(Ok(found)) => Checked::found(found.info()),
        Ok(Err(e)) => Checked::missing(format!("Couldn't get the file list: {e}")),
        Err(_) => Checked::missing("Couldn't get the file list: no peers answered yet".into()),
    }
}

/// A `file://` `.torrent`: read and inspected here. Its contents are not
/// valid, or it can't be read, is a failed check.
pub(super) async fn check_file(inner: &Inner, url: &Url) -> Result<(ProbeResult, Checked)> {
    let engine = inner
        .torrents
        .as_ref()
        .ok_or_else(|| Error::Unsupported("torrent downloads without a torrent folder".into()))?;
    let path = url
        .to_file_path()
        .map_err(|()| Error::InvalidUrl(url.to_string()))?;
    let bytes = tokio::fs::read(&path).await?;
    if bytes.len() > MAX_TORRENT_FILE {
        return Err(Error::Torrent(
            "the .torrent file is too large to be one".into(),
        ));
    }
    let found = engine.inspect(TorrentSource::Bytes(bytes)).await?;
    let probe = local_probe(
        url,
        filename::from_url(url).unwrap_or_else(|| "torrent".into()),
    );
    Ok((probe, Checked::found(found.info())))
}

/// An http(s) `.torrent`: fetched with the link's own connection and account,
/// then inspected. A failure is a note, since the link itself is reachable.
pub(super) async fn check_http(inner: &Inner, url: &Url, request: &RequestContext) -> Checked {
    let Some(engine) = &inner.torrents else {
        return Checked::default();
    };
    let bytes = match fetch_http(&inner.network, url, ProxyChoice::Auto, request.clone()).await {
        Ok(bytes) => bytes,
        Err(e) => return Checked::missing(format!("Couldn't get the file list: {e}")),
    };
    match engine.inspect(TorrentSource::Bytes(bytes)).await {
        Ok(found) => Checked::found(found.info()),
        Err(e) => Checked::missing(format!("Couldn't get the file list: {e}")),
    }
}

/// The probe of a link that is read locally: online, no size of its own.
fn local_probe(url: &Url, filename: String) -> ProbeResult {
    ProbeResult {
        final_url: url.clone(),
        size: None,
        accepts_ranges: false,
        etag: None,
        last_modified: None,
        content_type: None,
        filename,
    }
}
