//! Download manager tests: queueing, concurrency, pause/resume, restart
//! recovery and removal, against the shared local test server.

mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use common::{KIB, SIZE, Server, spawn_server};
use rdl_core::db::{self, Pool};
use rdl_core::http::SegmentConfig;
use rdl_core::manager::{
    DEFAULT_QUEUE, DownloadId, Event, Manager, ManagerConfig, NewDownload, NewPackage, Status,
};
use reqwest::Url;
use tokio::sync::broadcast;

const TIMEOUT: Duration = Duration::from_secs(15);

fn config() -> ManagerConfig {
    ManagerConfig {
        segment: SegmentConfig {
            max_connections: 4,
            min_segment_size: 256 * KIB,
            max_retries: 3,
            retry_base_delay: Duration::from_millis(10),
            progress_interval: Duration::from_millis(20),
        },
        save_interval: Duration::from_millis(30),
        rescan_interval: Duration::from_millis(200),
        user_agent: None,
        tools: None,
        archives_dir: None,
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    pool: Pool,
    manager: Manager,
    events: broadcast::Receiver<Event>,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(&dir.path().join("test.db")).await.unwrap();
        let manager = Manager::start(pool.clone(), config()).await.unwrap();
        let events = manager.subscribe();
        Self {
            dir,
            pool,
            manager,
            events,
        }
    }

    /// Stop this manager and start a fresh one on the same database.
    async fn restart(&mut self) {
        self.manager.shutdown().await;
        self.manager = Manager::start(self.pool.clone(), config()).await.unwrap();
        self.events = self.manager.subscribe();
    }

    fn dest_dir(&self) -> PathBuf {
        self.dir.path().join("downloads")
    }

    /// Add one package with the given URLs; returns the download ids in order.
    async fn add(&self, urls: &[Url]) -> Vec<DownloadId> {
        let package = self
            .manager
            .add_package(NewPackage {
                name: "test".into(),
                dest_dir: self.dest_dir(),
                queue_id: DEFAULT_QUEUE,
                downloads: urls.iter().cloned().map(NewDownload::new).collect(),
            })
            .await
            .unwrap();
        let packages = self.manager.list().await.unwrap();
        let package = packages.iter().find(|p| p.id == package).unwrap();
        package.downloads.iter().map(|d| d.id).collect()
    }

    async fn wait_for(&mut self, mut pred: impl FnMut(&Event) -> bool) -> Event {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                match self.events.recv().await {
                    Ok(event) if pred(&event) => return event,
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for event")
    }

    async fn wait_status(&mut self, id: DownloadId, want: Status) {
        self.wait_for(|e| {
            matches!(e, Event::StatusChanged { download_id, status, .. } if *download_id == id && *status == want)
        })
        .await;
    }

    async fn wait_progress_over(&mut self, id: DownloadId, bytes: u64) {
        self.wait_for(|e| {
            matches!(e, Event::Progress { download_id, downloaded, .. } if *download_id == id && *downloaded > bytes)
        })
        .await;
    }

    async fn saved_segments(&self, id: DownloadId) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM segments WHERE download_id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    fn part_file(&self, name: &str) -> PathBuf {
        self.dest_dir().join(format!("{name}.rdl.part"))
    }
}

fn url(base: &Url, path: &str) -> Url {
    base.join(path).unwrap()
}

fn assert_file(fx: &Fixture, name: &str, server: &Server) {
    common::assert_matches_server(&fx.dest_dir().join(name), server);
}

#[tokio::test]
async fn completes_a_package_and_numbers_colliding_names() {
    let (base, server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    // Both resolve to "test.bin".
    let ids = fx.add(&[url(&base, "file"), url(&base, "redirect")]).await;

    let mut pending: HashSet<_> = ids.iter().copied().collect();
    while !pending.is_empty() {
        if let Event::StatusChanged {
            download_id,
            status,
            error,
        } = fx
            .wait_for(|e| {
                matches!(
                    e,
                    Event::StatusChanged {
                        status: Status::Completed | Status::Failed,
                        ..
                    }
                )
            })
            .await
        {
            assert_eq!(
                status,
                Status::Completed,
                "download {download_id} failed: {error:?}"
            );
            pending.remove(&download_id);
        }
    }

    assert_file(&fx, "test.bin", &server);
    assert_file(&fx, "test (1).bin", &server);
    for id in ids {
        let d = fx.manager.get(id).await.unwrap();
        assert_eq!(
            (d.status, d.size, d.downloaded),
            (Status::Completed, Some(SIZE as u64), SIZE as u64)
        );
        assert_eq!(
            fx.saved_segments(id).await,
            0,
            "segments are cleared on completion"
        );
    }
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn queue_limit_caps_concurrent_downloads() {
    let (base, _server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    fx.manager.set_queue_limit(DEFAULT_QUEUE, 1).await.unwrap();
    let ids = fx
        .add(&[url(&base, "slow"), url(&base, "slow"), url(&base, "slow")])
        .await;

    let mut active = HashSet::new();
    let mut completed = HashSet::new();
    let mut max_active = 0;
    while completed.len() < ids.len() {
        let event = fx
            .wait_for(|e| matches!(e, Event::StatusChanged { .. }))
            .await;
        if let Event::StatusChanged {
            download_id,
            status,
            ..
        } = event
        {
            match status {
                Status::Active => {
                    active.insert(download_id);
                }
                Status::Completed => {
                    active.remove(&download_id);
                    completed.insert(download_id);
                }
                other => panic!("unexpected status {other:?}"),
            }
            max_active = max_active.max(active.len());
        }
    }
    assert_eq!(max_active, 1);
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn pause_saves_segments_and_resume_finishes_without_refetching() {
    let (base, server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    let id = fx.add(&[url(&base, "slow")]).await[0];

    fx.wait_progress_over(id, 512 * KIB).await;
    fx.manager.pause(id).await.unwrap();
    fx.wait_status(id, Status::Paused).await;

    let paused = fx.manager.get(id).await.unwrap();
    assert!(paused.downloaded > 0 && paused.downloaded < SIZE as u64);
    assert!(fx.saved_segments(id).await > 0);
    assert!(fx.part_file("test.bin").exists());

    let served_before = server.served();
    fx.manager.resume(id).await.unwrap();
    fx.wait_status(id, Status::Completed).await;

    assert_file(&fx, "test.bin", &server);
    assert!(!fx.part_file("test.bin").exists());
    assert!(
        server.served() - served_before < SIZE as u64,
        "resume refetched everything"
    );
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn shutdown_and_restart_resumes_automatically() {
    let (base, server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    let id = fx.add(&[url(&base, "slow")]).await[0];
    fx.wait_progress_over(id, 512 * KIB).await;

    fx.manager.shutdown().await;
    let stopped = fx.manager.get(id).await.unwrap();
    assert_eq!(
        stopped.status,
        Status::Queued,
        "shutdown should leave it queued"
    );
    assert!(fx.saved_segments(id).await > 0);

    let served_before = server.served();
    fx.restart().await;
    fx.wait_status(id, Status::Completed).await;

    assert_file(&fx, "test.bin", &server);
    assert!(
        server.served() - served_before < SIZE as u64,
        "restart refetched everything"
    );
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn downloads_left_active_by_a_crash_are_requeued() {
    let (base, server) = spawn_server().await;
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&dir.path().join("test.db")).await.unwrap();
    let dest = dir.path().join("downloads");
    sqlx::query("INSERT INTO packages (id, name, dest_dir, queue_id) VALUES (1, 'p', ?, 1)")
        .bind(dest.to_string_lossy().into_owned())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO downloads (id, package_id, kind, source_url, status) VALUES (1, 1, 'http', ?, 'active')")
        .bind(url(&base, "file").as_str())
        .execute(&pool)
        .await
        .unwrap();

    let manager = Manager::start(pool, config()).await.unwrap();
    let mut events = manager.subscribe();
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let Ok(Event::StatusChanged {
                status: Status::Completed,
                ..
            }) = events.recv().await
            {
                break;
            }
        }
    })
    .await
    .expect("crashed download never completed");

    common::assert_matches_server(&dest.join("test.bin"), &server);
    manager.shutdown().await;
}

#[tokio::test]
async fn removing_a_running_download_cleans_up_files_and_package() {
    let (base, _server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    let id = fx.add(&[url(&base, "slow")]).await[0];
    fx.wait_progress_over(id, 256 * KIB).await;

    fx.manager.remove(id, true).await.unwrap();

    assert!(!fx.part_file("test.bin").exists());
    assert!(fx.manager.list().await.unwrap().is_empty());
    assert!(fx.manager.get(id).await.is_err());
    fx.wait_for(|e| matches!(e, Event::PackageRemoved { .. }))
        .await;
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn failures_are_recorded_and_can_be_retried() {
    let (base, _server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    let id = fx.add(&[url(&base, "gone")]).await[0];

    let event = fx
        .wait_for(|e| {
            matches!(
                e,
                Event::StatusChanged {
                    status: Status::Failed,
                    ..
                }
            )
        })
        .await;
    let Event::StatusChanged { error, .. } = event else {
        unreachable!()
    };
    assert!(error.unwrap().contains("410"));
    assert_eq!(fx.manager.get(id).await.unwrap().status, Status::Failed);

    fx.manager.resume(id).await.unwrap();
    fx.wait_status(id, Status::Active).await;
    fx.wait_status(id, Status::Failed).await;
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn changed_remote_file_restarts_from_scratch_on_resume() {
    let (base, server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    let id = fx.add(&[url(&base, "slow")]).await[0];
    fx.wait_progress_over(id, 512 * KIB).await;
    fx.manager.pause(id).await.unwrap();
    fx.wait_status(id, Status::Paused).await;

    server.set_etag("\"v2\"");
    let served_before = server.served();
    fx.manager.resume(id).await.unwrap();
    fx.wait_status(id, Status::Completed).await;

    assert_file(&fx, "test.bin", &server);
    assert!(
        server.served() - served_before >= SIZE as u64,
        "stale segments were reused"
    );
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn pausing_a_queued_download_keeps_it_out_of_the_queue() {
    let (base, _server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    fx.manager.set_queue_limit(DEFAULT_QUEUE, 1).await.unwrap();
    let ids = fx.add(&[url(&base, "slow"), url(&base, "file")]).await;

    fx.manager.pause(ids[1]).await.unwrap();
    fx.wait_status(ids[1], Status::Paused).await;
    fx.wait_status(ids[0], Status::Completed).await;

    // Give the scheduler a chance to (wrongly) pick it up.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fx.manager.get(ids[1]).await.unwrap().status, Status::Paused);
    fx.manager.shutdown().await;
}
