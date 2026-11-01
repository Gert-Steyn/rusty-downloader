//! One running download: probe, pick a destination, run the engine, and keep
//! the database and event stream up to date until it stops.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::store::{self, DownloadRow, Resolved};
use super::{DownloadKind, Event, Inner, RunningJob, Status, StopReason, checksums};
use crate::http::{self, DownloadSpec, Outcome, ProbeResult, Progress, filename, segments};
use crate::ratelimit::RateLimiter;
use crate::{Error, Result, history};

const PART_SUFFIX: &str = ".rdl.part";

pub(super) fn part_path(dest: &Path) -> PathBuf {
    let mut name = OsString::from(dest.as_os_str());
    name.push(PART_SUFFIX);
    PathBuf::from(name)
}

/// Mark the download active and spawn its task.
pub(super) async fn start(inner: &Arc<Inner>, row: DownloadRow, queue_id: i64) -> Result<()> {
    let cancel = CancellationToken::new();
    let stop = Arc::new(Mutex::new(None));
    let (finished_tx, finished_rx) = watch::channel(false);
    let limit = Arc::new(RateLimiter::new(row.speed_limit()));
    inner.running().insert(
        row.id,
        RunningJob {
            queue_id,
            cancel: cancel.clone(),
            stop: Arc::clone(&stop),
            finished: finished_rx,
            limit: Arc::clone(&limit),
        },
    );
    // Registered as running first, so a pause from here on reaches the task;
    // one that came earlier has already moved it out of `queued`.
    if !store::mark_active(&inner.pool, row.id).await? {
        inner.running().remove(&row.id);
        return Ok(());
    }
    inner.emit(Event::StatusChanged {
        download_id: row.id,
        status: Status::Active,
        error: None,
    });
    tracing::info!(download = row.id, url = %row.source_url, "download started");

    let inner = Arc::clone(inner);
    tokio::spawn(async move {
        let mut last = None;
        let mut result = execute(&inner, &row, &cancel, &limit, &mut last).await;
        // A gallery is many files; a checksum belongs to one.
        if let (Ok(Outcome::Completed { path, .. }), false) =
            (&result, row.kind() == DownloadKind::Gallerydl)
        {
            if let Err(e) = checksums::check_finished(&inner, row.id, path, &cancel).await {
                result = Err(e);
            }
        }
        let list = match &result {
            Ok(Outcome::Completed { path, .. }) if row.kind() == DownloadKind::Http => {
                Some(path.clone())
            }
            _ => None,
        };
        let reason = *stop.lock().unwrap_or_else(|e| e.into_inner());
        // Completed or failed: either way it's done, as far as its queue goes.
        let ended = matches!(result, Ok(Outcome::Completed { .. }) | Err(_))
            && reason != Some(StopReason::Remove);
        if let Err(e) = finalize(&inner, &row, result, reason, last).await {
            tracing::error!(download = row.id, error = %e, "could not record download outcome");
        }
        if let Some(path) = list {
            if let Err(e) = checksums::apply_list(&inner, row.package_id, row.id, &path).await {
                tracing::warn!(download = row.id, error = %e, "could not read checksum list");
            }
        }
        // The queue it ended in: its package may have moved meanwhile.
        let queue = inner.running().remove(&row.id).map(|job| job.queue_id);
        if let (true, Some(queue)) = (ended, queue) {
            inner.ended_in().insert(queue);
        }
        let _ = finished_tx.send(true);
        inner.wake.notify_one();
    });
    Ok(())
}

/// Everything up to the engine returning. `last` receives the final engine
/// progress once the transfer has started, so the caller can persist it; it
/// stays `None` if we failed before touching the file, which keeps any saved
/// segments intact.
async fn execute(
    inner: &Arc<Inner>,
    row: &DownloadRow,
    cancel: &CancellationToken,
    limit: &Arc<RateLimiter>,
    last: &mut Option<Progress>,
) -> Result<Outcome> {
    // The file that failed its checksum last time goes; this is the retry.
    if row.checksum_ok == Some(0) && row.kind() != DownloadKind::Gallerydl {
        if let (Some(dest), Some(value)) = (&row.dest_path, &row.checksum_value) {
            super::remove_file_if_exists(Path::new(dest)).await?;
            store::set_checksum_ok(&inner.pool, row.id, value, None).await?;
        }
    }
    match row.kind.as_str() {
        "http" => {}
        "ytdlp" => return super::ytdlp_job::execute(inner, row, cancel).await,
        "gallerydl" => return super::gallerydl_job::execute(inner, row, cancel).await,
        other => return Err(Error::Unsupported(format!("{other} downloads"))),
    }
    let pool = &inner.pool;
    let package = store::get_package(pool, row.package_id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("package {}", row.package_id)))?;
    let url = Url::parse(&row.source_url)
        .map_err(|e| Error::InvalidUrl(format!("{}: {e}", row.source_url)))?;
    let spec = DownloadSpec::with_request(url, row.request());

    let probe = tokio::select! {
        _ = cancel.cancelled() => return Ok(Outcome::Paused),
        probe = http::probe(&inner.client, &spec) => probe?,
    };

    let mut resume_from = store::load_segments(pool, row.id).await?;
    if !resume_from.is_empty() && !same_remote_file(row, &probe) {
        tracing::warn!(
            download = row.id,
            "remote file changed since last run; restarting"
        );
        resume_from.clear();
        store::clear_segments(pool, row.id).await?;
    }

    let name = row
        .filename
        .clone()
        .unwrap_or_else(|| probe.filename.clone());
    let dest = {
        let _claim = inner.dest_lock.lock().await;
        let dest = match &row.dest_path {
            Some(path) => PathBuf::from(path),
            None => choose_destination(inner, Path::new(&package.dest_dir), &name).await?,
        };
        store::set_resolved(
            pool,
            row.id,
            &Resolved {
                resolved_url: probe.final_url.as_str(),
                filename: &name,
                dest_path: &dest,
                size: probe.size,
                etag: probe.etag.as_deref(),
                last_modified: probe.last_modified.as_deref(),
            },
        )
        .await?;
        dest
    };
    inner.emit(Event::Resolved {
        download_id: row.id,
        filename: name,
        size: probe.size,
    });

    let initial = Progress {
        downloaded: segments::total_written(&resume_from),
        total: probe.size,
        connections: 0,
        segments: resume_from.clone(),
    };
    let (tx, rx) = watch::channel(initial.clone());
    *last = Some(initial);
    let forwarder = tokio::spawn(forward_progress(Arc::clone(inner), row.id, rx));

    let outcome = http::download(
        &inner.client,
        &spec,
        &probe,
        &part_path(&dest),
        &dest,
        &resume_from,
        &inner.segment_config(limit),
        &tx,
        cancel,
    )
    .await;

    *last = Some(tx.borrow().clone());
    drop(tx);
    let _ = forwarder.await;
    outcome
}

/// Saved segments only apply if the server still has the same file.
fn same_remote_file(row: &DownloadRow, probe: &ProbeResult) -> bool {
    let same = |stored: &Option<String>, now: &Option<String>| match (stored, now) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    let size_ok = row.size.is_none_or(|s| probe.size == Some(s as u64));
    size_ok && same(&row.etag, &probe.etag) && same(&row.last_modified, &probe.last_modified)
}

/// First `name`, `name (1)`, ... in `dir` with no file, no part file, and no
/// other download already pointing at it. Caller holds `dest_lock`.
async fn choose_destination(inner: &Inner, dir: &Path, name: &str) -> Result<PathBuf> {
    for n in 0.. {
        let candidate = dir.join(filename::numbered(name, n));
        let on_disk = tokio::fs::try_exists(&candidate).await?
            || tokio::fs::try_exists(part_path(&candidate)).await?;
        if !on_disk && !store::dest_taken(&inner.pool, &candidate).await? {
            return Ok(candidate);
        }
    }
    unreachable!("unbounded range")
}

/// Turn engine snapshots into `Progress` events (with a smoothed speed) and
/// periodically persist segment state, until the engine drops its sender.
async fn forward_progress(inner: Arc<Inner>, id: i64, mut rx: watch::Receiver<Progress>) {
    const SMOOTHING: f64 = 0.3;
    let mut last_save = Instant::now();
    let mut previous: Option<(Instant, u64)> = None;
    let mut speed = 0.0_f64;

    while rx.changed().await.is_ok() {
        let progress = rx.borrow_and_update().clone();
        let now = Instant::now();
        if let Some((at, bytes)) = previous {
            let elapsed = now.duration_since(at).as_secs_f64();
            if elapsed > 0.0 {
                let instant = progress.downloaded.saturating_sub(bytes) as f64 / elapsed;
                speed = if speed == 0.0 {
                    instant
                } else {
                    SMOOTHING * instant + (1.0 - SMOOTHING) * speed
                };
            }
        }
        previous = Some((now, progress.downloaded));

        inner.emit(Event::Progress {
            download_id: id,
            downloaded: progress.downloaded,
            total: progress.total,
            speed_bps: speed as u64,
            connections: progress.connections,
            files: None,
        });

        if last_save.elapsed() >= inner.config.save_interval && !progress.segments.is_empty() {
            if let Err(e) = store::save_segments(&inner.pool, id, &progress.segments).await {
                tracing::warn!(download = id, error = %e, "could not save segment state");
            }
            last_save = now;
        }
    }
}

/// Record how the download ended.
async fn finalize(
    inner: &Inner,
    row: &DownloadRow,
    result: Result<Outcome>,
    reason: Option<StopReason>,
    last: Option<Progress>,
) -> Result<()> {
    if reason == Some(StopReason::Remove) {
        return Ok(()); // `Manager::remove` deletes the row once we signal.
    }
    let pool = &inner.pool;
    let id = row.id;
    let (status, error) = match result {
        Ok(Outcome::Completed { size, path }) => {
            store::mark_completed(pool, id, size).await?;
            record_history(inner, row, &path, size).await;
            (Status::Completed, None)
        }
        Ok(Outcome::Paused) => {
            if let Some(progress) = &last {
                store::save_segments(pool, id, &progress.segments).await?;
            }
            let status = match reason {
                Some(StopReason::Shutdown | StopReason::Requeue | StopReason::ScheduleClosed) => {
                    Status::Queued
                }
                _ => Status::Paused,
            };
            store::set_status(pool, id, status, None).await?;
            (status, None)
        }
        Err(err) => {
            if matches!(err, Error::RemoteChanged) {
                // The partial data is useless; the next attempt starts over.
                store::clear_segments(pool, id).await?;
            } else if matches!(err, Error::ChecksumMismatch { .. }) {
                // The whole file is there, it's just wrong; a retry starts over.
                let size = store::get_download(pool, id).await?.and_then(|r| r.size);
                store::clear_segments(pool, id).await?;
                if let Some(size) = size {
                    store::set_downloaded(pool, id, size as u64).await?;
                }
            } else if let Some(progress) = &last {
                store::save_segments(pool, id, &progress.segments).await?;
            }
            let message = err.to_string();
            tracing::warn!(download = id, error = %message, "download failed");
            store::set_status(pool, id, Status::Failed, Some(&message)).await?;
            (Status::Failed, Some(message))
        }
    };
    if status != Status::Failed {
        tracing::info!(download = id, status = status.as_str(), "download stopped");
    }
    inner.emit(Event::StatusChanged {
        download_id: id,
        status,
        error,
    });
    Ok(())
}

/// Add a completed download to the history; a failure is only logged.
pub(super) async fn record_history(inner: &Inner, row: &DownloadRow, path: &Path, size: u64) {
    let pool = &inner.pool;
    let package = store::get_package(pool, row.package_id)
        .await
        .ok()
        .flatten();
    let finished = history::Finished {
        url: &row.source_url,
        path,
        size: Some(size),
        kind: &row.kind,
        package: package.as_ref().map(|p| p.name.as_str()),
    };
    if let Err(e) = history::record(pool, &finished).await {
        tracing::warn!(download = row.id, error = %e, "could not record history");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path(Path::new(r"C:\dl\video.mp4")),
            PathBuf::from(r"C:\dl\video.mp4.rdl.part")
        );
    }
}
