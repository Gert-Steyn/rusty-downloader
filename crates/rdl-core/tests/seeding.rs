//! Seeding: a finished torrent keeps uploading until its limits are met.
//!
//! Like `torrent.rs`, a second librqbit session on 127.0.0.1 holds the data
//! and the manager downloads it from there. To make the manager's session
//! upload, a third session (the "leech") downloads from the manager's own
//! listening port. Nothing leaves the machine.

use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use librqbit::spawn_utils::BlockingSpawner;
use librqbit::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, ManagedTorrent, Session,
    SessionOptions, create_torrent,
};
use rdl_core::db::{self, Pool};
use rdl_core::manager::{
    DEFAULT_QUEUE, Download, DownloadId, Event, Manager, ManagerConfig, NewDownload, NewPackage,
    PackageId, Status,
};
use reqwest::Url;

const TIMEOUT: Duration = Duration::from_secs(20);
const MIB: usize = 1024 * 1024;

fn payload(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = fastrand::Rng::with_seed(seed);
    (0..len).map(|_| rng.u8(..)).collect()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn write_file(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path
}

/// A second session holding `content` complete, listening on loopback.
struct Seeder {
    _session: Arc<Session>,
    addr: SocketAddr,
    torrent: Vec<u8>,
    info_hash: String,
}

async fn seed(root: &Path, content: &Path) -> Seeder {
    let created = create_torrent(
        content,
        CreateTorrentOptions {
            name: None,
            trackers: Vec::new(),
            piece_length: Some(32 * 1024),
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap();
    let torrent = created.as_bytes().unwrap().to_vec();
    let addr = SocketAddr::from(([127, 0, 0, 1], free_port()));
    let session = Session::new_with_opts(
        root.join("seeder"),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            listen: Some(ListenerOptions {
                listen_addr: addr,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let handle: Arc<ManagedTorrent> = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.clone()),
            Some(AddTorrentOptions {
                output_folder: Some(content.parent().unwrap().to_string_lossy().into_owned()),
                overwrite: true,
                disable_trackers: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(TIMEOUT, handle.wait_until_completed())
        .await
        .expect("seeder verified its data")
        .unwrap();
    Seeder {
        info_hash: handle.info_hash().as_string(),
        _session: session,
        addr,
        torrent,
    }
}

/// The data every scenario downloads: one 2 MiB file, seeded from `root`.
struct Setup {
    root: tempfile::TempDir,
    seeder: Seeder,
    data: Vec<u8>,
    torrent_file: PathBuf,
    dest: PathBuf,
}

async fn setup() -> Setup {
    let root = tempfile::tempdir().unwrap();
    let data = payload(2 * MIB, 30);
    let content = write_file(&root.path().join("seed"), "film.bin", &data);
    let seeder = seed(root.path(), &content).await;
    let torrent_file = write_file(&root.path().join("links"), "film.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();
    Setup {
        root,
        seeder,
        data,
        torrent_file,
        dest,
    }
}

async fn open_pool(root: &Path) -> Pool {
    db::open(&root.join("test.db")).await.unwrap()
}

/// A manager on the state folder under `root`, with the seeding limits given.
async fn start(
    root: &Path,
    pool: Pool,
    peer: SocketAddr,
    seed_ratio: Option<f64>,
    seed_minutes: Option<u32>,
) -> Manager {
    Manager::start(
        pool,
        ManagerConfig {
            save_interval: Duration::from_millis(100),
            rescan_interval: Duration::from_millis(100),
            torrent_dir: Some(root.join("state")),
            torrent_test_peers: vec![peer],
            seed_ratio,
            seed_minutes,
            ..ManagerConfig::default()
        },
    )
    .await
    .unwrap()
}

/// Add the setup's torrent file as a download in a new package.
async fn add(manager: &Manager, setup: &Setup) -> DownloadId {
    let url = Url::from_file_path(&setup.torrent_file).unwrap();
    let package: PackageId = manager
        .add_package(NewPackage {
            name: "films".into(),
            dest_dir: setup.dest.clone(),
            queue_id: DEFAULT_QUEUE,
            downloads: vec![NewDownload::new(url)],
        })
        .await
        .unwrap();
    manager
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.id == package)
        .unwrap()
        .downloads[0]
        .id
}

/// Poll until `done` holds, failing with the download's state after the timeout.
async fn until(
    manager: &Manager,
    id: DownloadId,
    what: &str,
    done: impl Fn(&Download) -> bool,
) -> Download {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let download = manager.get(id).await.unwrap();
        if done(&download) {
            return download;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: status {:?}, seeding {}, uploaded {}, seeded {}s",
            download.status,
            download.seeding,
            download.uploaded,
            download.seeded_secs
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

async fn until_completed(manager: &Manager, id: DownloadId) -> Download {
    let download = until(manager, id, "completion", |d| {
        matches!(d.status, Status::Completed | Status::Failed)
    })
    .await;
    assert_eq!(download.status, Status::Completed, "{:?}", download.error);
    download
}

/// The manager's handle on the torrent, once it is paused (or not).
async fn wait_for_pause(manager: &Manager, info_hash: &str, paused: bool) -> Arc<ManagedTorrent> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let handle = manager
            .torrents()
            .unwrap()
            .find(info_hash)
            .await
            .unwrap()
            .expect("the torrent is in the session");
        if handle.is_paused() == paused {
            return handle;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for paused = {paused}"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// A third session that downloads the torrent from `peer`, so the peer uploads.
/// Returns the session (dropping it stops the download) and its folder.
async fn leech(root: &Path, torrent: &[u8], peer: SocketAddr) -> (Arc<Session>, PathBuf) {
    let dest = root.join("leech");
    let session = Session::new_with_opts(
        root.join("leech-session"),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.to_vec()),
            Some(AddTorrentOptions {
                output_folder: Some(dest.to_string_lossy().into_owned()),
                overwrite: true,
                disable_trackers: true,
                initial_peers: Some(vec![peer]),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(TIMEOUT, handle.wait_until_completed())
        .await
        .expect("the leech finished")
        .unwrap();
    (session, dest)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_torrent_keeps_seeding_while_its_ratio_is_short() {
    let s = setup().await;
    let manager = start(
        s.root.path(),
        open_pool(s.root.path()).await,
        s.seeder.addr,
        Some(1.0),
        None,
    )
    .await;
    let id = add(&manager, &s).await;

    let done = until_completed(&manager, id).await;
    assert!(done.seeding, "a finished torrent seeds");
    // Nobody leeches from us: the ratio stays at 0 and the torrent stays live.
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let later = manager.get(id).await.unwrap();
    assert_eq!(later.status, Status::Completed);
    assert!(later.seeding);
    assert_eq!(later.uploaded, 0);
    assert!(later.seeded_secs >= 1, "seeding time is counted");
    let handle = wait_for_pause(&manager, &s.seeder.info_hash, false).await;
    assert!(!handle.is_paused());
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn seeding_stops_when_the_ratio_is_reached() {
    let s = setup().await;
    let manager = start(
        s.root.path(),
        open_pool(s.root.path()).await,
        s.seeder.addr,
        Some(1.0),
        None,
    )
    .await;
    let mut events = manager.subscribe();
    let id = add(&manager, &s).await;
    until_completed(&manager, id).await;

    // A leech downloads the whole file from us: one full ratio.
    let listen = manager
        .torrents()
        .unwrap()
        .listen_addr()
        .await
        .unwrap()
        .expect("our session listens");
    let (_leech_session, leech_dir) = leech(s.root.path(), &s.seeder.torrent, listen).await;
    assert_eq!(std::fs::read(leech_dir.join("film.bin")).unwrap(), s.data);

    let stopped = until(&manager, id, "seeding to stop", |d| !d.seeding).await;
    assert_eq!(stopped.status, Status::Completed);
    assert!(
        stopped.uploaded >= s.data.len() as u64,
        "uploaded {} of {}",
        stopped.uploaded,
        s.data.len()
    );
    let handle = wait_for_pause(&manager, &s.seeder.info_hash, true).await;
    assert!(handle.is_paused(), "the torrent is paused once it stops");

    let mut heard_it_stop = false;
    while let Ok(event) = events.try_recv() {
        if let Event::Seeding {
            download_id,
            seeding: false,
            ..
        } = event
            && download_id == id
        {
            heard_it_stop = true;
        }
    }
    assert!(heard_it_stop, "no Seeding event said it stopped");
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn seeding_stops_when_the_time_limit_is_reached() {
    let s = setup().await;
    let pool = open_pool(s.root.path()).await;
    let manager = start(s.root.path(), pool.clone(), s.seeder.addr, None, Some(1)).await;
    let id = add(&manager, &s).await;
    let done = until_completed(&manager, id).await;
    assert!(done.seeding, "a time limit alone still seeds at first");

    // A minute is too long to wait for in a test: start just short of it.
    sqlx::query("UPDATE downloads SET seeded_secs = 59 WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let stopped = until(&manager, id, "seeding to stop", |d| !d.seeding).await;
    assert!(stopped.seeded_secs >= 60, "seeded {}s", stopped.seeded_secs);
    wait_for_pause(&manager, &s.seeder.info_hash, true).await;
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn seeding_can_be_stopped_and_started_by_hand() {
    let s = setup().await;
    let pool = open_pool(s.root.path()).await;
    let manager = start(s.root.path(), pool.clone(), s.seeder.addr, Some(1.0), None).await;
    let id = add(&manager, &s).await;
    assert!(until_completed(&manager, id).await.seeding);

    manager.stop_seeding(id).await.unwrap();
    let stopped = manager.get(id).await.unwrap();
    assert!(!stopped.seeding);
    assert_eq!(stopped.status, Status::Completed, "it stays finished");
    wait_for_pause(&manager, &s.seeder.info_hash, true).await;
    // Stopping again changes nothing.
    manager.stop_seeding(id).await.unwrap();

    manager.start_seeding(id).await.unwrap();
    assert!(manager.get(id).await.unwrap().seeding);
    wait_for_pause(&manager, &s.seeder.info_hash, false).await;

    // With its ratio already met, starting it gives it no limits of its own,
    // so it seeds until stopped by hand.
    manager.stop_seeding(id).await.unwrap();
    sqlx::query("UPDATE downloads SET uploaded = ? WHERE id = ?")
        .bind(s.data.len() as i64)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    manager.start_seeding(id).await.unwrap();
    let again = manager.get(id).await.unwrap();
    assert!(again.seeding);
    let options = again.torrent.expect("torrent options");
    assert!(options.custom_seed);
    assert_eq!(options.seed_ratio, None);
    assert_eq!(options.seed_minutes, None);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(
        manager.get(id).await.unwrap().seeding,
        "no limits, so it keeps seeding"
    );
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_torrent_gets_its_own_limits_and_they_apply_at_once() {
    let s = setup().await;
    let pool = open_pool(s.root.path()).await;
    let manager = start(s.root.path(), pool.clone(), s.seeder.addr, Some(1.0), None).await;
    let id = add(&manager, &s).await;
    until_completed(&manager, id).await;

    // Its own ratio of 2 is not reached, so it keeps seeding.
    sqlx::query("UPDATE downloads SET uploaded = ? WHERE id = ?")
        .bind(s.data.len() as i64)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    manager
        .set_torrent_seed_limits(id, true, Some(2.0), None)
        .await
        .unwrap();
    let download = manager.get(id).await.unwrap();
    assert!(download.seeding);
    assert_eq!(download.torrent.unwrap().seed_ratio, Some(2.0));

    // Ratio 1 is reached already, so the change stops it at once: no waiting.
    manager
        .set_torrent_seed_limits(id, true, Some(1.0), None)
        .await
        .unwrap();
    assert!(!manager.get(id).await.unwrap().seeding);

    // Not a limit at all.
    assert!(
        manager
            .set_torrent_seed_limits(id, true, Some(0.0), None)
            .await
            .is_err()
    );
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_network_hold_pauses_seeding_and_lifting_it_resumes() {
    let s = setup().await;
    let manager = start(
        s.root.path(),
        open_pool(s.root.path()).await,
        s.seeder.addr,
        None,
        None,
    )
    .await;
    let id = add(&manager, &s).await;
    until_completed(&manager, id).await;
    let hash = s.seeder.info_hash.clone();

    manager.set_network_hold(Some("the adapter is down".into()));
    wait_for_pause(&manager, &hash, true).await;
    let held = manager.get(id).await.unwrap();
    assert!(held.seeding, "held torrents still count as seeding");
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let during = manager.get(id).await.unwrap();
    assert_eq!(
        during.seeded_secs, held.seeded_secs,
        "no seeding time counts while held"
    );

    manager.set_network_hold(None);
    wait_for_pause(&manager, &hash, false).await;
    tokio::time::sleep(Duration::from_millis(3000)).await;
    let after = manager.get(id).await.unwrap();
    assert!(after.seeding);
    assert!(
        after.seeded_secs > during.seeded_secs,
        "seeding time counts again after the hold"
    );
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_seeding_torrent_seeds_again_after_a_restart() {
    let s = setup().await;
    let pool = open_pool(s.root.path()).await;
    let first = start(s.root.path(), pool.clone(), s.seeder.addr, None, None).await;
    let id = add(&first, &s).await;
    assert!(until_completed(&first, id).await.seeding);
    first.shutdown().await;
    drop(first);

    // The same database and state folder: the torrent comes back, and seeds.
    let second = start(s.root.path(), pool, s.seeder.addr, None, None).await;
    let restored = until(&second, id, "seeding to resume", |d| d.seeding).await;
    assert_eq!(restored.status, Status::Completed);
    wait_for_pause(&second, &s.seeder.info_hash, false).await;

    let listen = second
        .torrents()
        .unwrap()
        .listen_addr()
        .await
        .unwrap()
        .expect("our session listens");
    let (_leech_session, leech_dir) = leech(s.root.path(), &s.seeder.torrent, listen).await;
    assert_eq!(std::fs::read(leech_dir.join("film.bin")).unwrap(), s.data);
    second.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_seeding_torrent_leaves_nothing_seeding() {
    let s = setup().await;
    let manager = start(
        s.root.path(),
        open_pool(s.root.path()).await,
        s.seeder.addr,
        None,
        None,
    )
    .await;
    let id = add(&manager, &s).await;
    assert!(until_completed(&manager, id).await.seeding);

    manager.remove(id, true).await.unwrap();
    // Give the seeder a few checks: it must not add the torrent back.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(manager.get(id).await.is_err());
    let found = manager
        .torrents()
        .unwrap()
        .find(&s.seeder.info_hash)
        .await
        .unwrap();
    assert!(found.is_none(), "the removed torrent came back");
    manager.shutdown().await;
}
