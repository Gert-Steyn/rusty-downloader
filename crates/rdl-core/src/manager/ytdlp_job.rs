//! Running a `kind = 'ytdlp'` download: make sure the tools are there, run
//! yt-dlp, and mirror its progress into the database and event stream.
//!
//! Pause kills the process tree; resuming runs the same command again and
//! yt-dlp continues from its `.part` files. That only works if the output name
//! comes out the same, which it does: either the user's name or yt-dlp's
//! `title [id]` template, both fixed for a given video.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::store::{self, DownloadRow};
use super::{Event, Inner};
use crate::http::Outcome;
use crate::tools::ToolName;
use crate::ytdlp::{self, RunEvent, RunOutcome, RunSpec};
use crate::{Error, Result};

pub(super) async fn execute(
    inner: &Arc<Inner>,
    row: &DownloadRow,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let tools =
        inner.config.tools.as_ref().ok_or_else(|| {
            Error::Unsupported("yt-dlp downloads without the tool manager".into())
        })?;
    let pool = &inner.pool;
    let package = store::get_package(pool, row.package_id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("package {}", row.package_id)))?;
    let url = Url::parse(&row.source_url)
        .map_err(|e| Error::InvalidUrl(format!("{}: {e}", row.source_url)))?;

    // First use downloads the tools; a pause meanwhile just stops waiting.
    let exe = tokio::select! {
        _ = cancel.cancelled() => return Ok(Outcome::Paused),
        exe = tools.ensure(ToolName::Ytdlp) => exe?,
    };
    let ffmpeg = tokio::select! {
        _ = cancel.cancelled() => return Ok(Outcome::Paused),
        ffmpeg = tools.ensure(ToolName::Ffmpeg) => ffmpeg,
    };
    let ffmpeg_dir = match ffmpeg {
        Ok(path) => path.parent().map(Path::to_owned),
        Err(e) => {
            // yt-dlp still works, limited to formats that need no merging.
            tracing::warn!(download = row.id, error = %e, "ffmpeg unavailable");
            None
        }
    };

    let connection = inner.network.tool_connection(&url, row.proxy())?;
    let request = inner.network.with_account(&url, row.request()).0;
    let spec = RunSpec {
        exe,
        ffmpeg_dir,
        url,
        dest_dir: PathBuf::from(&package.dest_dir),
        name: row.filename.clone(),
        options: row.ytdlp_options().unwrap_or_default(),
        request,
        cookie_file: None,
        limit_rate: inner.tool_rate(row.speed_limit()),
        connection,
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let forwarder = tokio::spawn(forward(Arc::clone(inner), row.id, rx));
    let outcome = ytdlp::download(&spec, &tx, cancel).await;
    drop(tx);
    let downloaded = forwarder.await.unwrap_or(0);

    match outcome {
        Ok(RunOutcome::Completed { path, size }) => {
            // Extraction or remuxing can change the extension after the download.
            store::set_planned(pool, row.id, &path, Some(size)).await?;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            inner.emit(Event::Resolved {
                download_id: row.id,
                filename: name,
                size: Some(size),
            });
            Ok(Outcome::Completed { path, size })
        }
        Ok(RunOutcome::Paused) => {
            store::set_downloaded(pool, row.id, downloaded).await?;
            Ok(Outcome::Paused)
        }
        Err(e) => {
            store::set_downloaded(pool, row.id, downloaded).await?;
            Err(e)
        }
    }
}

/// Turn yt-dlp events into manager events; returns the last byte count.
async fn forward(inner: Arc<Inner>, id: i64, mut rx: mpsc::UnboundedReceiver<RunEvent>) -> u64 {
    let mut downloaded = 0;
    let mut last_save = Instant::now();
    while let Some(event) = rx.recv().await {
        match event {
            RunEvent::Planned { path, size } => {
                if let Err(e) = store::set_planned(&inner.pool, id, &path, size).await {
                    tracing::warn!(download = id, error = %e, "could not save planned file");
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                inner.emit(Event::Resolved {
                    download_id: id,
                    filename: name,
                    size,
                });
            }
            RunEvent::Progress {
                downloaded: done,
                total,
                speed_bps,
            } => {
                downloaded = done;
                inner.emit(Event::Progress {
                    download_id: id,
                    downloaded: done,
                    total,
                    speed_bps: speed_bps.unwrap_or(0),
                    connections: 1,
                    files: None,
                });

                if last_save.elapsed() >= inner.config.save_interval {
                    last_save = Instant::now();
                    if let Err(e) = store::set_downloaded(&inner.pool, id, done).await {
                        tracing::warn!(download = id, error = %e, "could not save progress");
                    }
                }
            }
        }
    }
    downloaded
}
