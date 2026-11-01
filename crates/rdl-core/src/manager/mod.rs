//! The download manager: owns the queue in SQLite, decides what runs, and
//! turns engine progress into persisted state and [`Event`]s.
//!
//! One scheduler task starts queued downloads whenever a queue has a free slot.
//! Every running download is its own task (see `job`). Control calls (`pause`,
//! `resume`, `remove`) update the database and either signal the running task
//! or wake the scheduler; they never block on a transfer except `remove`, which
//! waits for the task to let go of its files.

mod checksums;
mod gallerydl_job;
mod job;
mod queues;
mod store;
mod types;
mod ytdlp_job;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use std::time::Duration;

use tokio::sync::{Notify, broadcast, watch};
use tokio_util::sync::CancellationToken;

use crate::checksum::{self, ChecksumAlgo};
use crate::db::Pool;
use crate::http::{SegmentConfig, filename};
use crate::network::Network;
use crate::proxy::ProxyChoice;
use crate::ratelimit::{RateLimiter, Throttle};
use crate::tools::Tools;
use crate::{Error, Result, gallerydl, ytdlp};

pub use queues::{OnComplete, Queue, QueueChanges, QueueSchedule};
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
    /// Proxies; `None` connects directly.
    pub network: Option<Network>,
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
            network: None,
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
    network: Network,
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
    /// Queues where a download completed or failed since the queue last finished.
    ended_in: Mutex<HashSet<QueueId>>,
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
    /// Its queue's schedule window closed: queue it for the next window.
    ScheduleClosed,
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

    fn ended_in(&self) -> MutexGuard<'_, HashSet<QueueId>> {
        self.ended_in.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stop a running download for `reason` unless something else (a pause,
    /// a removal) is already stopping it. Returns whether it was asked.
    fn stop_if_undisturbed(&self, id: DownloadId, reason: StopReason) -> bool {
        let running = self.running();
        let Some(job) = running.get(&id) else {
            return false;
        };
        let mut stop = job.stop.lock().unwrap_or_else(|e| e.into_inner());
        if stop.is_some() {
            return false;
        }
        *stop = Some(reason);
        job.cancel.cancel();
        true
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
            network: config
                .network
                .clone()
                .unwrap_or_else(|| Network::direct(config.user_agent.as_deref())),
            pool,
            config,
            events,
            running: Mutex::new(HashMap::new()),
            dest_lock: tokio::sync::Mutex::new(()),
            wake: Notify::new(),
            shutdown: CancellationToken::new(),
            global_limit: Arc::new(RateLimiter::new(None)),
            ended_in: Mutex::new(HashSet::new()),
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

    /// Set a download's expected checksum from pasted text, or clear it with
    /// empty text. `algo` says which kind it is; `None` recognises it by its
    /// length. A finished file is checked at once, in the background, and
    /// one still downloading when it finishes. A mismatch fails the download
    /// (the file stays until a retry); a match after a mismatch completes it.
    pub async fn set_checksum(
        &self,
        id: DownloadId,
        algo: Option<ChecksumAlgo>,
        text: &str,
    ) -> Result<()> {
        let row = store::get_download(&self.inner.pool, id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("download {id}")))?;
        if row.kind() == DownloadKind::Gallerydl {
            return Err(Error::InvalidInput(
                "A gallery is many files, so it can't have one checksum.".into(),
            ));
        }
        let parsed = match text.trim() {
            "" => None,
            text => Some(checksum::parse_value(algo, text)?),
        };
        let checksum = parsed.as_ref().map(|(algo, value)| (*algo, value.as_str()));
        checksums::set(&self.inner, &row, checksum).await
    }

    /// Which connection a download uses. Takes effect when it next starts.
    pub async fn set_download_proxy(&self, id: DownloadId, proxy: ProxyChoice) -> Result<()> {
        if !store::set_proxy(&self.inner.pool, id, proxy).await? {
            return Err(Error::NotFound(format!("download {id}")));
        }
        tracing::info!(download = id, ?proxy, "download's connection changed");
        self.inner.emit(Event::ProxyChanged {
            download_id: id,
            proxy,
        });
        Ok(())
    }

    /// Every queue with its settings and what's in it, in order.
    pub async fn list_queues(&self) -> Result<Vec<Queue>> {
        let pool = &self.inner.pool;
        let counts = store::queue_counts(pool).await?;
        let now = chrono::Local::now().naive_local();
        Ok(store::queues(pool)
            .await?
            .into_iter()
            .map(|q| {
                let c = counts
                    .iter()
                    .find(|c| c.queue_id == q.id)
                    .copied()
                    .unwrap_or_default();
                let schedule = q.schedule();
                Queue {
                    id: q.id,
                    name: q.name.clone(),
                    max_active: q.max_active as u32,
                    open: schedule.is_open(now),
                    schedule,
                    on_complete: q.on_complete(),
                    packages: c.packages as u32,
                    active: c.active as u32,
                    queued: c.queued as u32,
                    paused: c.paused as u32,
                }
            })
            .collect())
    }

    /// A new queue, last in order, with no schedule and nothing to do when done.
    pub async fn add_queue(&self, name: &str) -> Result<QueueId> {
        let changes = QueueChanges {
            name: name.to_owned(),
            max_active: 3,
            schedule: QueueSchedule::default(),
            on_complete: OnComplete::None,
        }
        .validated()?;
        let id = store::insert_queue(&self.inner.pool, &changes.name, changes.max_active).await?;
        tracing::info!(queue = id, name = %changes.name, "queue added");
        self.inner.emit(Event::QueuesChanged);
        Ok(id)
    }

    pub async fn update_queue(&self, id: QueueId, changes: QueueChanges) -> Result<()> {
        let changes = changes.validated()?;
        if !store::update_queue(&self.inner.pool, id, &changes).await? {
            return Err(Error::NotFound(format!("queue {id}")));
        }
        tracing::info!(queue = id, ?changes, "queue changed");
        self.inner.emit(Event::QueuesChanged);
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Delete a queue; its packages move to the main queue. The main queue
    /// can't be deleted.
    pub async fn remove_queue(&self, id: QueueId) -> Result<()> {
        if id == DEFAULT_QUEUE {
            return Err(Error::InvalidInput(
                "The main queue can't be removed.".into(),
            ));
        }
        if !store::delete_queue(&self.inner.pool, id, DEFAULT_QUEUE).await? {
            return Err(Error::NotFound(format!("queue {id}")));
        }
        // Running downloads carry on, now counted in the main queue.
        for job in self.inner.running().values_mut() {
            if job.queue_id == id {
                job.queue_id = DEFAULT_QUEUE;
            }
        }
        tracing::info!(queue = id, "queue removed");
        self.inner.emit(Event::QueuesChanged);
        self.inner.emit(Event::PackagesReordered {
            queue_id: DEFAULT_QUEUE,
        });
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Move a package to the end of another queue. Its running downloads
    /// carry on and count towards the new queue from then on.
    pub async fn move_package_to_queue(&self, id: PackageId, queue: QueueId) -> Result<()> {
        let pool = &self.inner.pool;
        if store::queue(pool, queue).await?.is_none() {
            return Err(Error::NotFound(format!("queue {queue}")));
        }
        if store::get_package(pool, id).await?.is_none() {
            return Err(Error::NotFound(format!("package {id}")));
        }
        store::set_package_queue(pool, id, queue).await?;
        let downloads = store::package_download_ids(pool, id).await?;
        for (download, job) in self.inner.running().iter_mut() {
            if downloads.contains(download) {
                job.queue_id = queue;
            }
        }
        self.inner.emit(Event::QueuesChanged);
        self.inner
            .emit(Event::PackagesReordered { queue_id: queue });
        self.inner.wake.notify_one();
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
        } else {
            // It started meanwhile; a job is registered before it leaves `queued`.
            self.inner.request_stop(id, StopReason::Pause);
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

/// One pass over the queues: stop what runs in a queue whose window has
/// closed, fill free slots in open ones, and report queues that finished.
async fn schedule_once(inner: &Arc<Inner>) -> Result<()> {
    let now = chrono::Local::now().naive_local();
    for queue in store::queues(&inner.pool).await? {
        if inner.shutdown.is_cancelled() {
            return Ok(());
        }
        let running: Vec<DownloadId> = inner
            .running()
            .iter()
            .filter(|(_, job)| job.queue_id == queue.id)
            .map(|(id, _)| *id)
            .collect();
        if !queue.schedule().is_open(now) {
            for id in running {
                if inner.stop_if_undisturbed(id, StopReason::ScheduleClosed) {
                    tracing::info!(
                        download = id,
                        queue = queue.id,
                        "queue's window closed; stopping"
                    );
                }
            }
            continue;
        }
        let free = (queue.max_active.max(1) as usize).saturating_sub(running.len());
        let mut started = 0;
        if free > 0 {
            for row in store::next_queued(&inner.pool, queue.id, free).await? {
                job::start(inner, row, queue.id).await?;
                started += 1;
            }
        }
        if started == 0
            && running.is_empty()
            && inner.ended_in().contains(&queue.id)
            && store::pending_in_queue(&inner.pool, queue.id).await? == 0
        {
            inner.ended_in().remove(&queue.id);
            let on_complete = queue.on_complete();
            tracing::info!(queue = queue.id, ?on_complete, "queue finished");
            inner.emit(Event::QueueFinished {
                queue_id: queue.id,
                on_complete,
            });
        }
    }
    Ok(())
}
