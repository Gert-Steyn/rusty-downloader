//! The download engine.
//!
//! Range-capable servers get the segmented path: one task per connection, each
//! writing its own byte range into a pre-sized `.part` file through its own file
//! handle, with free connection slots filled by splitting the largest segment.
//! Everything else gets a single sequential stream.
//!
//! Pause is cooperative: cancelling the token makes every worker stop between
//! chunks and flush, and the final [`Progress`] carries the segment state needed
//! to resume later.

use std::collections::HashSet;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_RANGE, IF_RANGE, RANGE};
use reqwest::{Client, StatusCode};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::segments::{self, Segment};
use super::{DownloadSpec, ProbeResult, check_status, parse_content_range};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct SegmentConfig {
    pub max_connections: usize,
    /// Segments are never split below this size.
    pub min_segment_size: u64,
    /// Consecutive failures allowed per connection before the download fails.
    pub max_retries: u32,
    pub retry_base_delay: Duration,
    pub progress_interval: Duration,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            max_connections: 8,
            min_segment_size: 1024 * 1024,
            max_retries: 5,
            retry_base_delay: Duration::from_millis(500),
            progress_interval: Duration::from_millis(250),
        }
    }
}

/// Snapshot published while downloading. `segments` is what to persist for resume.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub connections: usize,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Completed {
        path: PathBuf,
        size: u64,
    },
    /// Cancelled by the caller; resume with the last published segments.
    Paused,
}

/// Download `spec` (already probed) into `part_path`, then move it to
/// `final_path`. Pass the segments from a previous [`Progress`] to resume, or an
/// empty list to start fresh.
#[allow(clippy::too_many_arguments)]
pub async fn download(
    client: &Client,
    spec: &DownloadSpec,
    probe: &ProbeResult,
    part_path: &Path,
    final_path: &Path,
    resume_from: &[Segment],
    config: &SegmentConfig,
    progress: &watch::Sender<Progress>,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    if fs::try_exists(final_path).await? {
        return Err(Error::DestinationExists(final_path.to_owned()));
    }
    if let Some(parent) = part_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let job = Job {
        client: client.clone(),
        spec: spec.clone(),
        probe: probe.clone(),
        part_path: part_path.to_owned(),
        config: config.clone(),
        cancel: cancel.child_token(),
    };
    let outcome = match probe.size {
        Some(size) if probe.is_segmentable() => segmented(job, size, resume_from, progress).await?,
        _ => single_stream(job, progress).await?,
    };
    if outcome == Outcome::Paused {
        return Ok(outcome);
    }

    let size = finish(part_path, final_path, probe.size).await?;
    tracing::info!(path = %final_path.display(), size, "download complete");
    Ok(Outcome::Completed {
        path: final_path.to_owned(),
        size,
    })
}

struct Job {
    client: Client,
    spec: DownloadSpec,
    probe: ProbeResult,
    part_path: PathBuf,
    config: SegmentConfig,
    /// Child of the caller's token, so an internal failure can stop the other
    /// workers without looking like a user pause.
    cancel: CancellationToken,
}

/// Returns `Completed` with no path; `download` does the final move.
async fn segmented(
    job: Job,
    size: u64,
    resume_from: &[Segment],
    progress: &watch::Sender<Progress>,
) -> Result<Outcome> {
    let initial = prepare_part_file(&job.part_path, size, resume_from).await?;
    let shared = Arc::new(Shared {
        job,
        segments: Mutex::new(initial),
    });

    let mut workers = JoinSet::new();
    let mut active = HashSet::new();
    let mut first_error = None;
    let mut ticker = tokio::time::interval(shared.job.config.progress_interval);

    fill_connections(&shared, &mut workers, &mut active);
    loop {
        tokio::select! {
            joined = workers.join_next() => {
                let Some(joined) = joined else { break };
                let (idx, result) = joined.unwrap_or_else(|e| (usize::MAX, Err(std::io::Error::other(e).into())));
                active.remove(&idx);
                match result {
                    Ok(()) if !shared.job.cancel.is_cancelled() => {
                        fill_connections(&shared, &mut workers, &mut active);
                    }
                    Ok(()) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, segment = idx, "segment failed; stopping download");
                        shared.job.cancel.cancel();
                        first_error.get_or_insert(e);
                    }
                }
            }
            _ = ticker.tick() => publish(&shared, size, active.len(), progress),
        }
    }
    publish(&shared, size, 0, progress);

    if let Some(e) = first_error {
        return Err(e);
    }
    if shared.job.cancel.is_cancelled() {
        return Ok(Outcome::Paused);
    }
    let written = segments::total_written(&shared.lock());
    if written != size {
        return Err(Error::SizeMismatch {
            expected: size,
            actual: written,
        });
    }
    Ok(Outcome::Completed {
        path: PathBuf::new(),
        size,
    })
}

struct Shared {
    job: Job,
    segments: Mutex<Vec<Segment>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Vec<Segment>> {
        // A worker can only panic between lock and unlock on a bug; the data is
        // plain numbers and still usable, so don't cascade the poison.
        self.segments.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Reuse the existing part file when the saved segments still describe it,
/// otherwise start over with one segment covering the whole file.
async fn prepare_part_file(
    path: &Path,
    size: u64,
    resume_from: &[Segment],
) -> Result<Vec<Segment>> {
    let existing_len = fs::metadata(path).await.map(|m| m.len()).ok();
    if existing_len == Some(size) && segments::covers(resume_from, size) {
        tracing::debug!(
            done = segments::total_written(resume_from),
            size,
            "resuming"
        );
        return Ok(resume_from.to_vec());
    }
    let file = File::create(path).await?;
    file.set_len(size).await?;
    Ok(vec![Segment::new(0, size - 1)])
}

/// Give every unfinished, unassigned segment a worker, then split the largest
/// segments until all connection slots are in use.
fn fill_connections(
    shared: &Arc<Shared>,
    workers: &mut JoinSet<(usize, Result<()>)>,
    active: &mut HashSet<usize>,
) {
    let max = shared.job.config.max_connections.max(1);
    let mut to_start = Vec::new();
    {
        let mut segs = shared.lock();
        for (idx, seg) in segs.iter().enumerate() {
            if active.len() + to_start.len() >= max {
                break;
            }
            if !seg.is_done() && !active.contains(&idx) {
                to_start.push(idx);
            }
        }
        while active.len() + to_start.len() < max {
            match segments::split_largest(&mut segs, shared.job.config.min_segment_size) {
                Some(idx) => to_start.push(idx),
                None => break,
            }
        }
    }
    for idx in to_start {
        active.insert(idx);
        let shared = Arc::clone(shared);
        workers.spawn(async move { (idx, segment_worker(&shared, idx).await) });
    }
}

fn publish(shared: &Shared, size: u64, connections: usize, progress: &watch::Sender<Progress>) {
    let segments = shared.lock().clone();
    progress.send_replace(Progress {
        downloaded: segments::total_written(&segments),
        total: Some(size),
        connections,
        segments,
    });
}

/// Fetch one segment to completion, retrying transient failures. Returns
/// `Ok(())` when the segment is done or the download was cancelled.
async fn segment_worker(shared: &Shared, idx: usize) -> Result<()> {
    let mut failures = 0;
    loop {
        let before = {
            let segs = shared.lock();
            if segs[idx].is_done() || shared.job.cancel.is_cancelled() {
                return Ok(());
            }
            segs[idx].next_offset()
        };

        let err = match fetch_range(shared, idx).await {
            Ok(()) => continue,
            Err(e) => e,
        };
        if shared.lock()[idx].next_offset() > before {
            failures = 0; // progress was made; only count consecutive failures
        }
        failures += 1;
        if !err.is_retryable() || failures > shared.job.config.max_retries {
            return Err(err);
        }
        let delay = backoff(shared.job.config.retry_base_delay, failures);
        tracing::debug!(error = %err, segment = idx, attempt = failures, ?delay, "retrying segment");
        tokio::select! {
            _ = shared.job.cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// One request for the rest of a segment. Stops early, without error, if the
/// segment is shrunk by a split or the download is cancelled.
async fn fetch_range(shared: &Shared, idx: usize) -> Result<()> {
    let job = &shared.job;
    let (offset, end) = {
        let seg = shared.lock()[idx];
        (seg.next_offset(), seg.end)
    };

    let mut request = job
        .client
        .get(job.probe.final_url.clone())
        .headers(job.spec.headers.clone())
        .header(RANGE, format!("bytes={offset}-{end}"))
        .header(ACCEPT_ENCODING, "identity");
    if let Some(validator) = job.probe.validator() {
        request = request.header(IF_RANGE, validator);
    }
    let response = tokio::select! {
        _ = job.cancel.cancelled() => return Ok(()),
        r = request.send() => r?,
    };

    let status = response.status();
    if status == StatusCode::OK {
        // A full body instead of 206 means either the validator no longer
        // matches (file changed) or the server stopped honouring ranges.
        return Err(if job.probe.validator().is_some() {
            Error::RemoteChanged
        } else {
            Error::RangeNotHonored
        });
    }
    check_status(status, response.url())?;
    let range_start = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_range)
        .map(|(start, _)| start);
    if range_start != Some(offset) {
        return Err(Error::RangeNotHonored);
    }

    let mut file = OpenOptions::new().write(true).open(&job.part_path).await?;
    file.seek(SeekFrom::Start(offset)).await?;
    let mut body = response.bytes_stream();
    let result = loop {
        let chunk = tokio::select! {
            _ = job.cancel.cancelled() => break Ok(()),
            chunk = body.next() => chunk,
        };
        let chunk = match chunk {
            Some(Ok(chunk)) => chunk,
            Some(Err(e)) => break Err(e.into()),
            None => break Err(Error::Incomplete),
        };

        // The segment's end may have moved down since the request was sent.
        let take = (chunk.len() as u64).min(shared.lock()[idx].remaining()) as usize;
        if take > 0 {
            file.write_all(&chunk[..take]).await?;
        }
        let mut segs = shared.lock();
        segs[idx].advance(take as u64);
        if segs[idx].is_done() {
            break Ok(());
        }
    };
    file.flush().await?;
    result
}

fn backoff(base: Duration, attempt: u32) -> Duration {
    let exp = base.saturating_mul(1 << attempt.saturating_sub(1).min(6));
    let capped = exp.min(Duration::from_secs(30));
    let jitter_ms = fastrand::u64(0..=base.as_millis().max(1) as u64);
    capped + Duration::from_millis(jitter_ms)
}

/// Sequential download for servers without range support or a known size.
/// Not resumable: every attempt starts again from byte 0.
async fn single_stream(job: Job, progress: &watch::Sender<Progress>) -> Result<Outcome> {
    let mut failures = 0;
    loop {
        match stream_once(&job, progress).await {
            Ok(outcome) => return Ok(outcome),
            Err(e) if e.is_retryable() && failures < job.config.max_retries => {
                failures += 1;
                let delay = backoff(job.config.retry_base_delay, failures);
                tracing::debug!(error = %e, attempt = failures, ?delay, "retrying stream from start");
                tokio::select! {
                    _ = job.cancel.cancelled() => return Ok(Outcome::Paused),
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            Err(e) => return Err(e),
        }
    }
}

async fn stream_once(job: &Job, progress: &watch::Sender<Progress>) -> Result<Outcome> {
    let response = tokio::select! {
        _ = job.cancel.cancelled() => return Ok(Outcome::Paused),
        r = job
            .client
            .get(job.probe.final_url.clone())
            .headers(job.spec.headers.clone())
            .header(ACCEPT_ENCODING, "identity")
            .send() => r?,
    };
    check_status(response.status(), response.url())?;

    let total = job.probe.size;
    let mut file = File::create(&job.part_path).await?;
    let mut body = response.bytes_stream();
    let mut written = 0u64;
    let mut last_publish = Instant::now();
    let report = |written: u64, connections: usize| Progress {
        downloaded: written,
        total,
        connections,
        segments: if written > 0 {
            vec![Segment {
                start: 0,
                end: written - 1,
                written,
            }]
        } else {
            Vec::new()
        },
    };

    let outcome = loop {
        let chunk = tokio::select! {
            _ = job.cancel.cancelled() => break Outcome::Paused,
            chunk = body.next() => chunk,
        };
        match chunk {
            Some(chunk) => {
                let chunk = chunk?;
                file.write_all(&chunk).await?;
                written += chunk.len() as u64;
                if last_publish.elapsed() >= job.config.progress_interval {
                    progress.send_replace(report(written, 1));
                    last_publish = Instant::now();
                }
            }
            None => {
                if total.is_some_and(|t| written < t) {
                    return Err(Error::Incomplete);
                }
                break Outcome::Completed {
                    path: PathBuf::new(),
                    size: written,
                };
            }
        }
    };
    file.flush().await?;
    progress.send_replace(report(written, 0));
    Ok(outcome)
}

/// Make the part file durable, check its size, and move it into place.
async fn finish(part_path: &Path, final_path: &Path, expected: Option<u64>) -> Result<u64> {
    OpenOptions::new()
        .write(true)
        .open(part_path)
        .await?
        .sync_all()
        .await?;
    let actual = fs::metadata(part_path).await?.len();
    if let Some(expected) = expected.filter(|&e| e != actual) {
        return Err(Error::SizeMismatch { expected, actual });
    }
    if fs::try_exists(final_path).await? {
        return Err(Error::DestinationExists(final_path.to_owned()));
    }
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::rename(part_path, final_path).await?;
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let base = Duration::from_millis(100);
        let first = backoff(base, 1);
        assert!(first >= base && first <= base * 2);
        let third = backoff(base, 3);
        assert!(third >= base * 4 && third <= base * 5);
        assert!(backoff(base, 50) <= Duration::from_secs(30) + base);
    }
}
