//! The download manager: owns the queue in SQLite, decides what runs, and
//! turns engine progress into persisted state and [`Event`]s.
//!
//! One scheduler task starts queued downloads whenever a queue has a free slot.
//! Every running download is its own task (see `job`). Control calls (`pause`,
//! `resume`, `remove`) update the database and either signal the running task
//! or wake the scheduler; they never block on a transfer except `remove`, which
//! waits for the task to let go of its files.

mod gallerydl_job;
mod job;
mod store;
mod types;
mod ytdlp_job;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use std::time::Duration;

use tokio::sync::{Notify, broadcast, watch};
use tokio_util::sync::CancellationToken;

use crate::db::Pool;
use crate::http::{self, SegmentConfig, filename};
use crate::ratelimit::{RateLimiter, Throttle};
use crate::tools::Tools;
use crate::{Error, Result, gallerydl, ytdlp};

pub use types::{
    DEFAULT_QUEUE, Download, DownloadId, DownloadKind, Event, FileCount, NewDownload, NewPackage,
    Package, PackageId, QueueId, Status,
};

#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub segment: SegmentConfig,
    /// How often a running download's segment state is written to SQLite.
    pub save_interval: Duration,
    /// Fallback re-check of the queues in case a wake-up was missed.
    pub rescan_interval: Duration,
    pub user_agent: Option<String>,
    /// Runs yt-dlp and gallery-dl downloads; without it they fail as unsupported.
    pub tools: Option<Tools>,
    /// Where gallery-dl's per-site download archives live; `None` runs
    /// without archives.
    pub archives_dir: Option<PathBuf>,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            segment: SegmentConfig::default(),
            save_interval: Duration::from_secs(2),
            rescan_interval: Duration::from_secs(5),
            user_agent: None,
            tools: None,
            archives_dir: None,
        }
    }
}

/// Cheap to clone; all clones drive the same manager.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

struct Inner {
    pool: Pool,
    client: reqwest::Client,
    config: ManagerConfig,
    events: broadcast::Sender<Event>,
    running: Mutex<HashMap<DownloadId, RunningJob>>,
    /// Serialises "pick a free destination name, then claim it in the DB".
    dest_lock: tokio::sync::Mutex<()>,
    wake: Notify,
    shutdown: CancellationToken,
    /// Connections per plain download, changeable while running; starts at
    /// `config.segment.max_connections`.
    max_connections: AtomicUsize,
    /// The speed limit all downloads share.
    global_limit: Arc<RateLimiter>,
}

struct RunningJob {
    queue_id: QueueId,
    cancel: CancellationToken,
    stop: Arc<Mutex<Option<StopReason>>>,
    finished: watch::Receiver<bool>,
    /// The download's own speed limit, changeable while it runs.
    limit: Arc<RateLimiter>,
}

/// Why a running download was cancelled; decides its status afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    Pause,
    /// Resumed again while it was still pausing: queue it straight back up.
    Requeue,
    /// App exit: queue it so it resumes on the next start.
    Shutdown,
    Remove,
}

impl Inner {
    /// Segment settings for a download starting now, whose own limit is `limit`.
    fn segment_config(&self, limit: &Arc<RateLimiter>) -> SegmentConfig {
        SegmentConfig {
            max_connections: self.max_connections.load(Ordering::Relaxed),
            throttle: Throttle::new([Arc::clone(&self.global_limit), Arc::clone(limit)]),
            ..self.config.segment.clone()
        }
    }

    /// The `--limit-rate` for a yt-dlp or gallery-dl run starting now: the
    /// download's own limit, or its share of the global one if smaller. The
    /// tools can't be slowed while they run, so this is fixed at the start.
    fn tool_rate(&self, own: Option<u64>) -> Option<u64> {
        let share = self.global_limit.rate().map(|global| {
            let running = self.running().len().max(1) as u64;
            (global / running).max(crate::ratelimit::MIN_RATE)
        });
        match (own, share) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn emit(&self, event: Event) {
        // No subscribers is fine.
        let _ = self.events.send(event);
    }

    fn running(&self) -> MutexGuard<'_, HashMap<DownloadId, RunningJob>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ask a running download to stop. Returns a handle to wait on, or `None`
    /// if it isn't running.
    fn request_stop(&self, id: DownloadId, reason: StopReason) -> Option<watch::Receiver<bool>> {
        let running = self.running();
        let job = running.get(&id)?;
        *job.stop.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
        job.cancel.cancel();
        Some(job.finished.clone())
    }
}

impl Manager {
    /// Recover from the previous run and start scheduling.
    pub async fn start(pool: Pool, config: ManagerConfig) -> Result<Self> {
        let recovered = store::requeue_interrupted(&pool).await?;
        if recovered > 0 {
            tracing::info!(
                recovered,
                "re-queued downloads interrupted by the last exit"
            );
        }
        // Tool runs that were killed with the app left their cookies behind.
        tokio::task::spawn_blocking(crate::request::sweep_cookie_files);
        let (events, _) = broadcast::channel(1024);
        let inner = Arc::new(Inner {
            max_connections: AtomicUsize::new(config.segment.max_connections),
            client: http::client(config.user_agent.as_deref())?,
            pool,
            config,
            events,
            running: Mutex::new(HashMap::new()),
            dest_lock: tokio::sync::Mutex::new(()),
            wake: Notify::new(),
            shutdown: CancellationToken::new(),
            global_limit: Arc::new(RateLimiter::new(None)),
        });
        tokio::spawn(scheduler(Arc::clone(&inner)));
        Ok(Self { inner })
    }

    /// The speed limit all downloads share, in bytes per second; `None` lifts it.
    /// Plain downloads feel it at once; yt-dlp and gallery-dl runs from
    /// their next start.
    pub fn set_speed_limit(&self, limit: Option<u64>) {
        if self.inner.global_limit.rate() != limit {
            tracing::info!(?limit, "global speed limit");
        }
        self.inner.global_limit.set_rate(limit);
    }

    pub fn speed_limit(&self) -> Option<u64> {
        self.inner.global_limit.rate()
    }

    /// A download's own speed limit (`None`: only the global one applies).
    /// Takes effect at once for a running plain download.
    pub async fn set_download_speed_limit(&self, id: DownloadId, limit: Option<u64>) -> Result<()> {
        let limit = limit.map(|l| l.max(crate::ratelimit::MIN_RATE));
        if !store::set_speed_limit(&self.inner.pool, id, limit).await? {
            return Err(Error::NotFound(format!("download {id}")));
        }
        if let Some(job) = self.inner.running().get(&id) {
            job.limit.set_rate(limit);
        }
        self.inner.emit(Event::SpeedLimitChanged {
            download_id: id,
            speed_limit: limit,
        });
        Ok(())
    }

    /// Downloads running right now (including ones that are stopping).
    pub fn running_count(&self) -> usize {
        self.inner.running().len()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub async fn add_package(&self, mut new: NewPackage) -> Result<PackageId> {
        for download in &mut new.downloads {
            download.filename = download.filename.as_deref().map(filename::sanitize);
        }
        let id = store::insert_package(&self.inner.pool, &new).await?;
        tracing::info!(
            package = id,
            downloads = new.downloads.len(),
            "package added"
        );
        self.inner.emit(Event::PackageAdded { package_id: id });
        self.inner.wake.notify_one();
        Ok(id)
    }

    pub async fn list(&self) -> Result<Vec<Package>> {
        store::list_packages(&self.inner.pool).await
    }

    pub async fn get(&self, id: DownloadId) -> Result<Download> {
        store::get_download(&self.inner.pool, id)
            .await?
            .map(|row| row.to_download())
            .ok_or_else(|| Error::NotFound(format!("download {id}")))
    }

    pub async fn set_queue_limit(&self, queue: QueueId, max_active: u32) -> Result<()> {
        if !store::set_queue_limit(&self.inner.pool, queue, max_active).await? {
            return Err(Error::NotFound(format!("queue {queue}")));
        }
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Connections for plain downloads that start from now on; running ones
    /// keep theirs.
    pub fn set_max_connections(&self, connections: usize) {
        self.inner
            .max_connections
            .store(connections.max(1), Ordering::Relaxed);
    }

    /// Move a package within its queue, before `before` (or to the end).
    /// Packages start in this order, after priority.
    pub async fn move_package(&self, id: PackageId, before: Option<PackageId>) -> Result<()> {
        let pool = &self.inner.pool;
        let package = store::get_package(pool, id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("package {id}")))?;
        let mut order = store::queue_package_ids(pool, package.queue_id).await?;
        order.retain(|p| *p != id);
        let at = before
            .and_then(|b| order.iter().position(|p| *p == b))
            .unwrap_or(order.len());
        order.insert(at, id);
        store::set_package_order(pool, &order).await?;
        self.inner.emit(Event::PackagesReordered {
            queue_id: package.queue_id,
        });
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Pause a queued or running download. Running ones stop between chunks and
    /// save their segments first.
    pub async fn pause(&self, id: DownloadId) -> Result<()> {
        if self.inner.request_stop(id, StopReason::Pause).is_some() {
            return Ok(());
        }
        let pool = &self.inner.pool;
        if store::transition(pool, id, &[Status::Queued], Status::Paused).await? {
            self.status_changed(id, Status::Paused, None);
        }
        Ok(())
    }

    /// Queue a paused or failed download again.
    pub async fn resume(&self, id: DownloadId) -> Result<()> {
        {
            let running = self.inner.running();
            if let Some(job) = running.get(&id) {
                let mut stop = job.stop.lock().unwrap_or_else(|e| e.into_inner());
                if *stop == Some(StopReason::Pause) {
                    *stop = Some(StopReason::Requeue);
                }
                return Ok(());
            }
        }
        let from = [Status::Paused, Status::Failed, Status::Canceled];
        if store::transition(&self.inner.pool, id, &from, Status::Queued).await? {
            self.status_changed(id, Status::Queued, None);
            self.inner.wake.notify_one();
        }
        Ok(())
    }

    pub async fn pause_all(&self) -> Result<()> {
        for id in
            store::ids_with_status(&self.inner.pool, &[Status::Queued, Status::Active]).await?
        {
            self.pause(id).await?;
        }
        Ok(())
    }

    pub async fn resume_all(&self) -> Result<()> {
        for id in
            store::ids_with_status(&self.inner.pool, &[Status::Paused, Status::Failed]).await?
        {
            self.resume(id).await?;
        }
        Ok(())
    }

    /// Remove a download, stopping it first. Its `.part` file is always deleted;
    /// with `delete_files`, a completed file is deleted too. A gallery deletes
    /// the files it wrote whatever its status (a failed one still finished
    /// some), then its folder if that leaves it empty. An emptied package is
    /// removed as well.
    pub async fn remove(&self, id: DownloadId, delete_files: bool) -> Result<()> {
        if let Some(mut finished) = self.inner.request_stop(id, StopReason::Remove) {
            let _ = finished.wait_for(|done| *done).await;
        }
        let pool = &self.inner.pool;
        let row = store::get_download(pool, id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("download {id}")))?;

        let completed = row.status() == Status::Completed;
        if let Some(dest) = row.dest_path.as_deref().map(std::path::Path::new) {
            match row.kind() {
                DownloadKind::Http => remove_file_if_exists(&job::part_path(dest)).await?,
                DownloadKind::Ytdlp if !completed => {
                    for leftover in ytdlp::partial_files(dest) {
                        remove_file_if_exists(&leftover).await?;
                    }
                }
                DownloadKind::Gallerydl if !completed => {
                    for leftover in gallerydl::partial_files(dest) {
                        remove_file_if_exists(&leftover).await?;
                    }
                }
                _ => {}
            }
            // A gallery that stopped part-way still has whole files of its own.
            if delete_files && row.kind() == DownloadKind::Gallerydl {
                for file in store::files_of(pool, id).await? {
                    remove_file_if_exists(&file).await?;
                }
                // Only succeeds when nothing else is in there.
                let _ = tokio::fs::remove_dir(dest).await;
            } else if delete_files && completed {
                remove_file_if_exists(dest).await?;
            }
        }

        store::delete_download(pool, id).await?;
        tracing::info!(download = id, delete_files, "download removed");
        self.inner.emit(Event::DownloadRemoved {
            download_id: id,
            package_id: row.package_id,
        });
        if store::delete_package_if_empty(pool, row.package_id).await? {
            self.inner.emit(Event::PackageRemoved {
                package_id: row.package_id,
            });
        }
        Ok(())
    }

    pub async fn remove_package(&self, id: PackageId, delete_files: bool) -> Result<()> {
        let pool = &self.inner.pool;
        if store::get_package(pool, id).await?.is_none() {
            return Err(Error::NotFound(format!("package {id}")));
        }
        for download in store::package_download_ids(pool, id).await? {
            self.remove(download, delete_files).await?;
        }
        // Covers a package that had no downloads to begin with.
        if store::delete_package_if_empty(pool, id).await? {
            self.inner.emit(Event::PackageRemoved { package_id: id });
        }
        Ok(())
    }

    /// Stop scheduling and stop every running download, saving their state and
    /// leaving them queued for the next start. Safe to call more than once.
    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        let ids: Vec<DownloadId> = self.inner.running().keys().copied().collect();
        let waits: Vec<_> = ids
            .into_iter()
            .filter_map(|id| self.inner.request_stop(id, StopReason::Shutdown))
            .collect();
        tracing::info!(stopping = waits.len(), "manager shutting down");
        for mut finished in waits {
            let _ = finished.wait_for(|done| *done).await;
        }
    }

    fn status_changed(&self, id: DownloadId, status: Status, error: Option<String>) {
        self.inner.emit(Event::StatusChanged {
            download_id: id,
            status,
            error,
        });
    }
}

async fn remove_file_if_exists(path: &std::path::Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

async fn scheduler(inner: Arc<Inner>) {
    loop {
        if let Err(e) = schedule_once(&inner).await {
            tracing::error!(error = %e, "scheduling failed");
        }
        tokio::select! {
            _ = inner.shutdown.cancelled() => break,
            _ = inner.wake.notified() => {}
            _ = tokio::time::sleep(inner.config.rescan_interval) => {}
        }
    }
    tracing::debug!("scheduler stopped");
}

async fn schedule_once(inner: &Arc<Inner>) -> Result<()> {
    for queue in store::queues(&inner.pool).await? {
        if inner.shutdown.is_cancelled() {
            return Ok(());
        }
        let running = inner
            .running()
            .values()
            .filter(|job| job.queue_id == queue.id)
            .count();
        let free = (queue.max_active.max(1) as usize).saturating_sub(running);
        if free == 0 {
            continue;
        }
        for row in store::next_queued(&inner.pool, queue.id, free).await? {
            job::start(inner, row, queue.id).await?;
        }
    }
    Ok(())
}
