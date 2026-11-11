//! Torrent downloads against a local seeder: a second librqbit session on
//! 127.0.0.1 that holds the complete data. Nothing leaves the machine: DHT and
//! trackers are off, and the manager is given the seeder's address as a peer.

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
    DEFAULT_QUEUE, Download, DownloadId, DownloadKind, Manager, ManagerConfig, NewDownload,
    NewPackage, PackageId, Status,
};
use reqwest::Url;

const TIMEOUT: Duration = Duration::from_secs(20);
const MIB: usize = 1024 * 1024;

/// Deterministic bytes, so a failure shows up as a mismatch, not as luck.
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

/// A second session holding `content` (a file, or a folder for a multi-file
/// torrent) complete, and listening on loopback.
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
    // The data is already where the torrent expects it: a file's folder, or
    // a multi-file torrent's own folder (librqbit uses that folder as-is).
    let folder = if content.is_dir() {
        content
    } else {
        content.parent().unwrap()
    };
    let handle: Arc<ManagedTorrent> = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.clone()),
            Some(AddTorrentOptions {
                output_folder: Some(folder.to_string_lossy().into_owned()),
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

/// Write `payload` as `name` in `dir`, creating `dir`.
fn write_file(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path
}

fn config(state: &Path, peer: SocketAddr) -> ManagerConfig {
    ManagerConfig {
        save_interval: Duration::from_millis(100),
        rescan_interval: Duration::from_millis(100),
        torrent_dir: Some(state.to_path_buf()),
        torrent_test_peers: vec![peer],
        ..ManagerConfig::default()
    }
}

async fn open_pool(root: &Path) -> Pool {
    db::open(&root.join("test.db")).await.unwrap()
}

/// Add one torrent download to a new package in `dest`.
async fn add_torrent(manager: &Manager, dest: &Path, url: Url) -> DownloadId {
    let package: PackageId = manager
        .add_package(NewPackage {
            name: "torrents".into(),
            dest_dir: dest.to_path_buf(),
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
            "timed out waiting for {what}: status {:?}, downloaded {}, error {:?}",
            download.status,
            download.downloaded,
            download.error
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

async fn until_completed(manager: &Manager, id: DownloadId) -> Download {
    let download = until(manager, id, "completion", |d| {
        matches!(d.status, Status::Completed | Status::Failed)
    })
    .await;
    assert_eq!(
        download.status,
        Status::Completed,
        "error: {:?}",
        download.error
    );
    download
}

#[tokio::test(flavor = "multi_thread")]
async fn torrent_file_downloads_and_matches_the_source() {
    let root = tempfile::tempdir().unwrap();
    let data = payload(3 * MIB, 1);
    let content = write_file(&root.path().join("seed"), "payload.bin", &data);
    let seeder = seed(root.path(), &content).await;

    let torrent_file = write_file(
        &root.path().join("links"),
        "payload.torrent",
        &seeder.torrent,
    );
    let url = Url::from_file_path(&torrent_file).unwrap();
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();

    let pool = open_pool(root.path()).await;
    let manager = Manager::start(pool, config(&root.path().join("state"), seeder.addr))
        .await
        .unwrap();
    let id = add_torrent(&manager, &dest, url).await;

    let download = until_completed(&manager, id).await;
    assert_eq!(download.kind, "torrent");
    assert_eq!(download.dest_path, Some(dest.join("payload.bin")));
    assert_eq!(download.size, Some(data.len() as u64));
    assert_eq!(std::fs::read(dest.join("payload.bin")).unwrap(), data);
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_file_torrent_lands_in_its_own_folder() {
    let root = tempfile::tempdir().unwrap();
    let album = root.path().join("seed").join("album");
    let a = payload(MIB, 2);
    let b = payload(300 * 1024, 3);
    write_file(&album, "a.bin", &a);
    write_file(&album, "b.bin", &b);
    let seeder = seed(root.path(), &album).await;

    let torrent_file = write_file(&root.path().join("links"), "album.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();

    let manager = Manager::start(
        open_pool(root.path()).await,
        config(&root.path().join("state"), seeder.addr),
    )
    .await
    .unwrap();
    let id = add_torrent(&manager, &dest, Url::from_file_path(&torrent_file).unwrap()).await;

    let download = until_completed(&manager, id).await;
    assert_eq!(download.dest_path, Some(dest.join("album")));
    assert_eq!(std::fs::read(dest.join("album").join("a.bin")).unwrap(), a);
    assert_eq!(std::fs::read(dest.join("album").join("b.bin")).unwrap(), b);
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_then_resume_completes_the_torrent() {
    let root = tempfile::tempdir().unwrap();
    let data = payload(3 * MIB, 4);
    let content = write_file(&root.path().join("seed"), "slow.bin", &data);
    let seeder = seed(root.path(), &content).await;

    let torrent_file = write_file(&root.path().join("links"), "slow.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();

    let manager = Manager::start(
        open_pool(root.path()).await,
        config(&root.path().join("state"), seeder.addr),
    )
    .await
    .unwrap();
    // Slow enough (512 KiB/s) to pause part-way through.
    manager.set_speed_limit(Some(512 * 1024));
    let id = add_torrent(&manager, &dest, Url::from_file_path(&torrent_file).unwrap()).await;

    let total = data.len() as u64;
    let running = until(&manager, id, "some progress", |d| {
        d.downloaded > 0 && d.status == Status::Active
    })
    .await;
    assert!(running.downloaded < total);

    manager.pause(id).await.unwrap();
    let paused = until(&manager, id, "the pause", |d| d.status == Status::Paused).await;
    assert!(paused.downloaded < total, "paused after everything arrived");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(manager.get(id).await.unwrap().status, Status::Paused);

    manager.set_speed_limit(None);
    manager.resume(id).await.unwrap();
    let download = until_completed(&manager, id).await;
    assert_eq!(download.downloaded, total);
    assert_eq!(std::fs::read(dest.join("slow.bin")).unwrap(), data);
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_picks_the_torrent_up_again() {
    let root = tempfile::tempdir().unwrap();
    let data = payload(3 * MIB, 5);
    let content = write_file(&root.path().join("seed"), "again.bin", &data);
    let seeder = seed(root.path(), &content).await;

    let torrent_file = write_file(&root.path().join("links"), "again.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();
    let state = root.path().join("state");
    let pool = open_pool(root.path()).await;

    // First run: stopped part-way by an app exit.
    let first = Manager::start(pool.clone(), config(&state, seeder.addr))
        .await
        .unwrap();
    first.set_speed_limit(Some(512 * 1024));
    let id = add_torrent(&first, &dest, Url::from_file_path(&torrent_file).unwrap()).await;
    until(&first, id, "some progress", |d| d.downloaded > 0).await;
    first.shutdown().await;
    assert_eq!(first.get(id).await.unwrap().status, Status::Queued);
    drop(first);

    // Second run: it re-joins the torrent and finishes it.
    let second = Manager::start(pool, config(&state, seeder.addr))
        .await
        .unwrap();
    let download = until_completed(&second, id).await;
    assert_eq!(std::fs::read(dest.join("again.bin")).unwrap(), data);
    assert_eq!(download.dest_path, Some(dest.join("again.bin")));
    second.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_with_files_deletes_the_data_and_forgets_the_torrent() {
    let root = tempfile::tempdir().unwrap();
    let data = payload(MIB, 6);
    let content = write_file(&root.path().join("seed"), "gone.bin", &data);
    let seeder = seed(root.path(), &content).await;

    let torrent_file = write_file(&root.path().join("links"), "gone.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();
    let state = root.path().join("state");

    let manager = Manager::start(open_pool(root.path()).await, config(&state, seeder.addr))
        .await
        .unwrap();
    let id = add_torrent(&manager, &dest, Url::from_file_path(&torrent_file).unwrap()).await;
    until_completed(&manager, id).await;
    assert!(dest.join("gone.bin").exists());

    manager.remove(id, true).await.unwrap();
    assert!(!dest.join("gone.bin").exists(), "the data is still there");
    assert!(manager.get(id).await.is_err());
    // The engine keeps a `.torrent` copy per torrent; none may be left.
    let leftovers: Vec<_> = std::fs::read_dir(state.join("session"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".torrent"))
        .collect();
    assert!(leftovers.is_empty(), "the engine still knows the torrent");
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn magnet_link_fetches_its_metadata_from_the_peer() {
    let root = tempfile::tempdir().unwrap();
    let data = payload(MIB + 1234, 7);
    let content = write_file(&root.path().join("seed"), "magnet.bin", &data);
    let seeder = seed(root.path(), &content).await;

    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();
    let manager = Manager::start(
        open_pool(root.path()).await,
        config(&root.path().join("state"), seeder.addr),
    )
    .await
    .unwrap();
    let magnet = Url::parse(&format!("magnet:?xt=urn:btih:{}", seeder.info_hash)).unwrap();
    assert_eq!(DownloadKind::for_url(&magnet), DownloadKind::Torrent);
    let id = add_torrent(&manager, &dest, magnet).await;

    let download = until_completed(&manager, id).await;
    assert_eq!(download.dest_path, Some(dest.join("magnet.bin")));
    assert_eq!(std::fs::read(dest.join("magnet.bin")).unwrap(), data);
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn torrents_fail_as_unsupported_without_a_torrent_folder() {
    let root = tempfile::tempdir().unwrap();
    let torrent_file = write_file(&root.path().join("links"), "x.torrent", b"d4:infod e");
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();

    let manager = Manager::start(open_pool(root.path()).await, ManagerConfig::default())
        .await
        .unwrap();
    let id = add_torrent(&manager, &dest, Url::from_file_path(&torrent_file).unwrap()).await;
    let download = until(&manager, id, "failure", |d| d.status == Status::Failed).await;
    assert!(
        download
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("torrent downloads"),
        "{:?}",
        download.error
    );
    manager.shutdown().await;
}

#[test]
fn links_pick_the_torrent_kind() {
    let kind = |link: &str| DownloadKind::for_url(&Url::parse(link).unwrap());
    let torrent = [
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=x",
        "https://example.com/files/ubuntu.torrent",
        "HTTP://example.com/Ubuntu.TORRENT?download=1",
        "file:///C:/Downloads/a.torrent",
    ];
    for link in torrent {
        assert_eq!(kind(link), DownloadKind::Torrent, "{link}");
    }
    let other = [
        ("https://example.com/file.zip", DownloadKind::Http),
        ("https://example.com/torrents/", DownloadKind::Http),
        ("ftp://example.com/a.torrent", DownloadKind::Ftp),
    ];
    for (link, expected) in other {
        assert_eq!(kind(link), expected, "{link}");
    }
    assert_eq!(DownloadKind::Torrent.as_str(), "torrent");
    assert_eq!(DownloadKind::parse("torrent"), DownloadKind::Torrent);
}
