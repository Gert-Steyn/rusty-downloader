//! Running a `kind = 'gallerydl'` download: make sure gallery-dl is there,
//! run it into the gallery's folder, and mirror the files it reports into
//! the database and event stream.
//!
//! The whole gallery is one download. Its folder is `<package folder>/<name>`
//! (the name the LinkGrabber showed, editable there), so a second run lands
//! in the same place and gallery-dl skips what's done. Pause kills the
//! process tree; resuming runs it again, and the skipped files are counted
//! as done straight away.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::store::{self, DownloadRow};
use super::{Event, FileCount, Inner};
use crate::gallerydl::{self, RunEvent, RunOutcome, RunSpec};
use crate::http::{Outcome, filename};
use crate::tools::ToolName;
use crate::{Error, Result};

/// Progress events at most this often; skipped files arrive by the hundred.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Speed is averaged over the files finished this recently.
const SPEED_WINDOW: Duration = Duration::from_secs(5);

pub(super) async fn execute(
    inner: &Arc<Inner>,
    row: &DownloadRow,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let tools = inner.config.tools.as_ref().ok_or_else(|| {
        Error::Unsupported("gallery-dl downloads without the tool manager".into())
    })?;
    let pool = &inner.pool;
    let package = store::get_package(pool, row.package_id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("package {}", row.package_id)))?;
    let url = Url::parse(&row.source_url)
        .map_err(|e| Error::InvalidUrl(format!("{}: {e}", row.source_url)))?;

    let exe = tokio::select! {
        _ = cancel.cancelled() => return Ok(Outcome::Paused),
        exe = tools.ensure(ToolName::Gallerydl) => exe?,
    };
    // Only needed to convert Pixiv animations; not worth a 90 MB install here.
    let ffmpeg_dir = tools
        .path(ToolName::Ffmpeg)
        .await
        .ok()
        .flatten()
        .and_then(|p| p.parent().map(Path::to_owned));
    let available = match gallerydl::supported_flags(&exe).await {
        Ok(flags) => Some(flags),
        Err(e) => {
            tracing::warn!(error = %e, "gallery-dl self-check failed; passing every option");
            None
        }
    };

    let dir = match &row.dest_path {
        Some(path) => PathBuf::from(path),
        None => Path::new(&package.dest_dir).join(folder_name(row, &url)),
    };
    store::set_planned(pool, row.id, &dir, None).await?;
    inner.emit(Event::Resolved {
        download_id: row.id,
        filename: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: row.size.map(|s| s as u64),
    });

    let connection = inner.network.tool_connection(&url, row.proxy())?;
    let request = inner.network.with_account(&url, row.request()).0;
    let spec = RunSpec {
        exe,
        ffmpeg_dir,
        archive: inner
            .config
            .archives_dir
            .as_deref()
            .map(|d| gallerydl::archive_path(d, &url)),
        url,
        dir: dir.clone(),
        options: row.gallerydl_options().unwrap_or_default(),
        available,
        request,
        cookie_file: None,
        limit_rate: inner.tool_rate(row.speed_limit()),
        connection,
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let total = row.files_total.map(|t| t as u32);
    let before = (
        row.files_done.unwrap_or(0) as u32,
        row.downloaded.max(0) as u64,
    );
    let forwarder = tokio::spawn(forward(Arc::clone(inner), row.id, total, before, rx));
    let outcome = gallerydl::download(&spec, &tx, cancel).await;
    drop(tx);
    let (files, bytes, new_files) = forwarder.await.unwrap_or((0, 0, 0));
    store::set_files_done(pool, row.id, files, bytes).await?;
    // A rerun keeps its count across pauses: a resumed run skips the files
    // this run already fetched, so they're only counted here.
    let rerun_new = {
        let mut reruns = inner.reruns();
        reruns.get_mut(&row.id).map(|count| {
            *count += new_files;
            *count
        })
    };

    match outcome {
        Ok(RunOutcome::Completed { files, bytes }) => {
            if files == 0 {
                return Err(Error::Tool(
                    "gallery-dl finished without finding any files".into(),
                ));
            }
            // Now the count is known for certain.
            store::set_files_total(pool, row.id, files).await?;
            if let Some(new_files) = rerun_new {
                // The rerun is over: report it before `finalize` reports
                // the status, so listeners see the result first.
                inner.reruns().remove(&row.id);
                inner.emit(Event::GalleryChecked {
                    download_id: row.id,
                    new_files,
                });
            }

            Ok(Outcome::Completed {
                path: dir,
                size: bytes,
            })
        }
        Ok(RunOutcome::Paused) => Ok(Outcome::Paused),
        Err(e) => Err(e),
    }
}

/// The gallery's folder: the name chosen in the LinkGrabber, else the title,
/// else the last part of the URL.
fn folder_name(row: &DownloadRow, url: &Url) -> String {
    let name = row
        .filename
        .clone()
        .or_else(|| row.title.clone())
        .or_else(|| filename::from_url(url))
        .unwrap_or_else(|| "Gallery".into());
    filename::sanitize(&name)
}

/// Turn file reports into manager events; returns the file and byte counts
/// to save, and how many of the files were new (not skipped) in this run. A
/// resumed run first re-reports the files it skips, so until it passes
/// `before` (the last run's counts) those are shown instead: the count never
/// goes backwards.
async fn forward(
    inner: Arc<Inner>,
    id: i64,
    total: Option<u32>,
    before: (u32, u64),
    mut rx: mpsc::UnboundedReceiver<RunEvent>,
) -> (u32, u64, u32) {
    let (mut files, mut bytes) = (0u32, 0u64);
    let mut new_files = 0u32;
    let mut recent: VecDeque<(Instant, u64)> = VecDeque::new();
    let started = Instant::now();
    let mut last_emit: Option<Instant> = None;
    let mut last_save = Instant::now();
    loop {
        let event = rx.recv().await;
        let finished = event.is_none();
        if let Some(RunEvent::File {
            path,
            size,
            skipped,
        }) = event
        {
            files += 1;
            bytes += size;
            if !skipped {
                new_files += 1;
                recent.push_back((Instant::now(), size));
            }
            // A file skipped because it's already in the gallery's folder is
            // still one of its files (from an earlier run or download), and
            // counts as done, so "delete files" removes it too. One skipped
            // because of the archive alone isn't on disk (size 0).
            if (!skipped || size > 0)
                && let Err(e) = store::add_file(&inner.pool, id, &path, size).await
            {
                tracing::warn!(download = id, error = %e, "could not record file");
            }
        }
        let (shown_files, shown_bytes) = if files >= before.0 {
            (files, bytes)
        } else {
            before
        };
        let due = last_emit.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL);
        if (due || finished) && files > 0 {
            last_emit = Some(Instant::now());
            while recent
                .front()
                .is_some_and(|(at, _)| at.elapsed() > SPEED_WINDOW)
            {
                recent.pop_front();
            }
            let window = started.elapsed().min(SPEED_WINDOW).as_secs_f64().max(1.0);
            let speed = recent.iter().map(|(_, s)| s).sum::<u64>() as f64 / window;
            inner.emit(Event::Progress {
                download_id: id,
                downloaded: shown_bytes,
                total: None,
                speed_bps: speed as u64,
                connections: 1,
                files: Some(FileCount {
                    done: shown_files,
                    total: total.map(|t| t.max(shown_files)),
                }),
            });
            if last_save.elapsed() >= inner.config.save_interval {
                last_save = Instant::now();
                if let Err(e) =
                    store::set_files_done(&inner.pool, id, shown_files, shown_bytes).await
                {
                    tracing::warn!(download = id, error = %e, "could not save progress");
                }
            }
        }
        if finished {
            return (shown_files, shown_bytes, new_files);
        }
    }
}
