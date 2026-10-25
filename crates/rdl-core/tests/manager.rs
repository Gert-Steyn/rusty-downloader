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
use rdl_core::request::{Cookie, RequestContext};
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
            throttle: Default::default(),
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
        self.add_downloads(urls.iter().cloned().map(NewDownload::new).collect())
            .await
    }

    async fn add_downloads(&self, downloads: Vec<NewDownload>) -> Vec<DownloadId> {
        let package = self
            .manager
            .add_package(NewPackage {
                name: "test".into(),
                dest_dir: self.dest_dir(),
                queue_id: DEFAULT_QUEUE,
                downloads,
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

    let history = rdl_core::history::list(&fx.pool, None, 10).await.unwrap();
    let mut names: Vec<&str> = history.iter().map(|h| h.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["test (1).bin", "test.bin"]);
    assert!(
        history
            .iter()
            .all(|h| h.exists && h.package.as_deref() == Some("test"))
    );
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn packages_can_be_reordered_and_connections_changed_at_runtime() {
    let (base, _server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    fx.manager.set_queue_limit(DEFAULT_QUEUE, 1).await.unwrap();
    fx.manager.set_max_connections(1);
    // The slow one takes the only slot; it started after the change, so it
    // uses one connection.
    let a = fx.add(&[url(&base, "slow")]).await[0];
    let event = fx
        .wait_for(|e| matches!(e, Event::Progress { download_id, connections, .. } if *download_id == a && *connections > 0))
        .await;
    assert!(matches!(event, Event::Progress { connections: 1, .. }));
    let b = fx.add(&[url(&base, "named/b.bin")]).await[0];
    let c = fx.add(&[url(&base, "named/c.bin")]).await[0];
    let mut packages = Vec::new();
    for id in [a, b, c] {
        packages.push(fx.manager.get(id).await.unwrap().package_id);
    }
    let [pa, pb, pc] = packages[..] else {
        unreachable!()
    };

    // c jumps ahead of b, so it starts once a (the slow one) is out of the way.
    fx.manager.move_package(pc, Some(pb)).await.unwrap();
    let order: Vec<i64> = fx
        .manager
        .list()
        .await
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(order, [pa, pc, pb]);
    // Moving to the end.
    fx.manager.move_package(pa, None).await.unwrap();
    let order: Vec<i64> = fx
        .manager
        .list()
        .await
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(order, [pc, pb, pa]);

    // Freeing the slot starts c before b.
    fx.manager.pause(a).await.unwrap();
    let started = fx
        .wait_for(|e| matches!(e, Event::StatusChanged { status: Status::Active, download_id, .. } if *download_id == b || *download_id == c))
        .await;
    assert!(matches!(started, Event::StatusChanged { download_id, .. } if download_id == c));
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
    assert_eq!(fx.manager.running_count(), 1);

    fx.manager.remove(id, true).await.unwrap();

    assert_eq!(fx.manager.running_count(), 0, "remove waits for the task");
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

/// Wait until download `id` completes or fails; returns the final status.
async fn finished(fx: &mut Fixture, id: DownloadId) -> Status {
    let event = fx
        .wait_for(|e| {
            matches!(e, Event::StatusChanged { download_id, status: Status::Completed | Status::Failed, .. } if *download_id == id)
        })
        .await;
    let Event::StatusChanged { status, .. } = event else {
        unreachable!()
    };
    status
}

#[tokio::test]
async fn browser_cookies_and_referer_reach_only_their_host() {
    let (base, server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    let request = RequestContext {
        referer: Some(format!("{base}page")),
        user_agent: Some("TestBrowser/1".into()),
        cookies: vec![Cookie {
            name: "sid".into(),
            value: "ok".into(),
            domain: "127.0.0.1".into(),
            host_only: true,
            path: "/".into(),
            secure: false,
            http_only: true,
            expires: None,
        }],
    };
    let with_request = |path: &str| NewDownload {
        request: request.clone(),
        ..NewDownload::new(url(&base, path))
    };

    // Signed in: probe and every segment send the cookie and referer.
    let id = fx.add_downloads(vec![with_request("private")]).await[0];
    assert_eq!(finished(&mut fx, id).await, Status::Completed);
    assert_file(&fx, "test.bin", &server);
    {
        let seen = server.private_cookies.lock().unwrap();
        assert!(seen.len() > 1, "expected probe plus segment requests");
        assert!(seen.iter().all(|c| c.as_deref() == Some("sid=ok")));
    }
    let stored: Option<String> =
        sqlx::query_scalar("SELECT request_json FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert_eq!(
        stored, None,
        "cookies are dropped once the download is done"
    );

    // Without them, the server refuses.
    let id = fx.add(&[url(&base, "private")]).await[0];
    assert_eq!(finished(&mut fx, id).await, Status::Failed);

    // A redirect to another host doesn't take the cookie along.
    server.private_cookies.lock().unwrap().clear();
    let id = fx.add_downloads(vec![with_request("to-localhost")]).await[0];
    assert_eq!(finished(&mut fx, id).await, Status::Failed);
    assert_eq!(*server.private_cookies.lock().unwrap(), [None]);
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn the_global_speed_limit_slows_downloads() {
    let (base, server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    // 4 MiB at 2 MiB/s: about 2 s (less the bucket's quarter second).
    fx.manager.set_speed_limit(Some(2 * 1024 * 1024));
    assert_eq!(fx.manager.speed_limit(), Some(2 * 1024 * 1024));
    let started = std::time::Instant::now();
    let id = fx.add(&[url(&base, "file")]).await[0];
    assert_eq!(finished(&mut fx, id).await, Status::Completed);
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(1500) && took < Duration::from_secs(5),
        "took {took:?}"
    );
    assert_file(&fx, "test.bin", &server);

    // Lifted, the same file comes in at full speed.
    fx.manager.set_speed_limit(None);
    let started = std::time::Instant::now();
    let id = fx.add(&[url(&base, "named/again.bin")]).await[0];
    assert_eq!(finished(&mut fx, id).await, Status::Completed);
    assert!(started.elapsed() < Duration::from_millis(1500));
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn a_downloads_own_limit_changes_while_it_runs() {
    let (base, server) = spawn_server().await;
    let mut fx = Fixture::new().await;
    let id = fx.add(&[url(&base, "file")]).await[0];
    // Slowed right away: 4 MiB at 256 KiB/s would take 16 s.
    fx.manager
        .set_download_speed_limit(id, Some(256 * 1024))
        .await
        .unwrap();
    fx.wait_for(|e| {
        matches!(e, Event::SpeedLimitChanged { download_id, speed_limit: Some(_) } if *download_id == id)
    })
    .await;
    assert_eq!(
        fx.manager.get(id).await.unwrap().speed_limit,
        Some(256 * 1024)
    );
    fx.wait_status(id, Status::Active).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let partway = fx.manager.get(id).await.unwrap();
    assert_ne!(partway.status, Status::Completed, "the limit didn't hold");

    // Lifted while running: it finishes promptly.
    let started = std::time::Instant::now();
    fx.manager.set_download_speed_limit(id, None).await.unwrap();
    assert_eq!(finished(&mut fx, id).await, Status::Completed);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(fx.manager.get(id).await.unwrap().speed_limit, None);
    assert_file(&fx, "test.bin", &server);

    assert!(
        fx.manager
            .set_download_speed_limit(9999, Some(1024))
            .await
            .is_err()
    );
    fx.manager.shutdown().await;
}
