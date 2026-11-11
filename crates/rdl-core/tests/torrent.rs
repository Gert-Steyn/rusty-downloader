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
use rdl_core::linkgrabber::{Availability, GrabbedLink, LinkGrabber, LinkId, LinkSource};
use rdl_core::manager::{
    DEFAULT_QUEUE, Download, DownloadId, DownloadKind, Manager, ManagerConfig, NewDownload,
    NewPackage, PackageId, Status,
};
use rdl_core::network::Network;
use rdl_core::torrent::{TorrentConfig, TorrentEngine, TorrentOptions, TorrentSource};
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

/// A torrent engine that asks `peer` for metadata and pieces, on its own
/// state folder. Like the manager's, but for a LinkGrabber used alone.
fn engine(state: &Path, peer: Option<SocketAddr>) -> TorrentEngine {
    TorrentEngine::new(
        TorrentConfig {
            state_dir: state.to_path_buf(),
            test_peers: peer.into_iter().collect(),
        },
        Network::direct(None),
    )
}

/// A manager and a LinkGrabber on one database, sharing the torrent engine,
/// as in the app.
async fn manager_and_grabber(root: &Path, peer: SocketAddr) -> (Manager, LinkGrabber) {
    let pool = open_pool(root).await;
    let manager = Manager::start(pool.clone(), config(&root.join("state"), peer))
        .await
        .unwrap();
    let grabber = LinkGrabber::start(pool, None, None, manager.torrents())
        .await
        .unwrap();
    (manager, grabber)
}

/// Poll the LinkGrabber until the link's check has finished.
async fn link_settled(grabber: &LinkGrabber, id: LinkId) -> GrabbedLink {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let link = grabber
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|l| l.id == id)
            .expect("the link is still in the LinkGrabber");
        if !matches!(
            link.availability,
            Availability::Unknown | Availability::Checking
        ) {
            return link;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the check: {link:?}"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// The first download of a package.
async fn first_download(manager: &Manager, package: PackageId) -> DownloadId {
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

/// Two files: `a.bin` (1 MiB) and `b.bin` (300 KiB), seeded as `album`.
async fn seed_album(root: &Path) -> (Seeder, Vec<u8>, Vec<u8>) {
    let album = root.join("seed").join("album");
    let a = payload(MIB, 20);
    let b = payload(300 * 1024, 21);
    write_file(&album, "a.bin", &a);
    write_file(&album, "b.bin", &b);
    (seed(root, &album).await, a, b)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_magnet_in_the_linkgrabber_gets_its_file_list() {
    let root = tempfile::tempdir().unwrap();
    let (seeder, _, b) = seed_album(root.path()).await;
    let grabber = LinkGrabber::start(
        open_pool(root.path()).await,
        None,
        None,
        Some(engine(&root.path().join("state"), Some(seeder.addr))),
    )
    .await
    .unwrap();

    let magnet = Url::parse(&format!(
        "magnet:?xt=urn:btih:{}&dn=Album+link",
        seeder.info_hash
    ))
    .unwrap();
    let added = grabber.add(vec![magnet], LinkSource::Manual).await.unwrap();
    let link = link_settled(&grabber, added.added[0].id).await;

    assert_eq!(link.availability, Availability::Online, "{:?}", link.error);
    assert_eq!(link.error, None);
    assert_eq!(link.kind, DownloadKind::Torrent);
    let info = link.torrent.expect("the file list");
    assert_eq!(info.info_hash, seeder.info_hash);
    assert_eq!(info.name, "album");
    assert_eq!(info.files.len(), 2);
    assert_eq!(info.files[0].path, "a.bin");
    assert_eq!(info.files[1].path, "b.bin");
    assert_eq!(info.files[1].size, b.len() as u64);
    assert_eq!(link.size, Some(info.size));
    assert_eq!(link.torrent_options, Some(TorrentOptions::default()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cached_magnet_needs_no_peer() {
    let root = tempfile::tempdir().unwrap();
    let (seeder, _, _) = seed_album(root.path()).await;
    let state = root.path().join("state");
    let magnet = format!("magnet:?xt=urn:btih:{}", seeder.info_hash);
    let hash = seeder.info_hash.clone();

    // The first inspection asks the seeder, and keeps the metadata.
    let first = engine(&state, Some(seeder.addr));
    let found = first
        .inspect(TorrentSource::Magnet(magnet.clone()))
        .await
        .unwrap();
    assert_eq!(found.files.len(), 2);
    assert!(
        state
            .join("metadata")
            .join(format!("{hash}.torrent"))
            .exists()
    );

    // The seeder is gone, and the engine has no peers at all. The copy answers.
    drop(seeder);
    let later = engine(&state, None);
    let again = tokio::time::timeout(
        Duration::from_secs(5),
        later.inspect(TorrentSource::Magnet(magnet)),
    )
    .await
    .expect("answered from the copy, without waiting for peers")
    .unwrap();
    assert_eq!(again.info_hash, hash);
    assert_eq!(again.name, found.name);
    assert_eq!(later.inspect_cached(&hash).unwrap().size, found.size);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_chosen_files_download() {
    let root = tempfile::tempdir().unwrap();
    let (seeder, _, b) = seed_album(root.path()).await;
    let torrent_file = write_file(&root.path().join("links"), "album.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();
    let (manager, grabber) = manager_and_grabber(root.path(), seeder.addr).await;

    let added = grabber
        .add(
            vec![Url::from_file_path(&torrent_file).unwrap()],
            LinkSource::Manual,
        )
        .await
        .unwrap();
    let link = link_settled(&grabber, added.added[0].id).await;
    assert_eq!(link.availability, Availability::Online, "{:?}", link.error);
    grabber
        .set_torrent_files(&[link.id], Some(vec![1]))
        .await
        .unwrap();
    let link = link_settled(&grabber, link.id).await;
    assert_eq!(
        link.size,
        Some(b.len() as u64),
        "the size is the chosen file's"
    );

    let packages = grabber.confirm(&[link.id], &dest, &manager).await.unwrap();
    let id = first_download(&manager, packages[0]).await;
    let download = until_completed(&manager, id).await;
    assert_eq!(download.size, Some(b.len() as u64));
    assert_eq!(std::fs::read(dest.join("album").join("b.bin")).unwrap(), b);
    // librqbit creates a file that wasn't chosen empty, and writes nothing to it.
    let skipped = dest.join("album").join("a.bin");
    assert_eq!(
        std::fs::metadata(&skipped).unwrap().len(),
        0,
        "the file that wasn't chosen was downloaded"
    );
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn torrent_files_lists_each_file_with_its_selection() {
    let root = tempfile::tempdir().unwrap();
    let (seeder, _, b) = seed_album(root.path()).await;
    let torrent_file = write_file(&root.path().join("links"), "album.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();
    let (manager, grabber) = manager_and_grabber(root.path(), seeder.addr).await;

    let added = grabber
        .add(
            vec![Url::from_file_path(&torrent_file).unwrap()],
            LinkSource::Manual,
        )
        .await
        .unwrap();
    let link = link_settled(&grabber, added.added[0].id).await;
    grabber
        .set_torrent_files(&[link.id], Some(vec![1]))
        .await
        .unwrap();
    let packages = grabber.confirm(&[link.id], &dest, &manager).await.unwrap();
    let id = first_download(&manager, packages[0]).await;
    until_completed(&manager, id).await;

    let files = manager.torrent_files(id).await.unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "a.bin");
    assert!(!files[0].selected);
    assert_eq!(files[1].path, "b.bin");
    assert!(files[1].selected);
    assert_eq!(files[1].size, b.len() as u64);
    assert_eq!(files[1].downloaded, b.len() as u64);
    assert_eq!(
        manager.get(id).await.unwrap().torrent,
        Some(TorrentOptions {
            only_files: Some(vec![1])
        })
    );
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_renamed_multi_file_torrent_lands_in_the_renamed_folder() {
    let root = tempfile::tempdir().unwrap();
    let (seeder, a, b) = seed_album(root.path()).await;
    let torrent_file = write_file(&root.path().join("links"), "album.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();
    let (manager, grabber) = manager_and_grabber(root.path(), seeder.addr).await;

    let added = grabber
        .add(
            vec![Url::from_file_path(&torrent_file).unwrap()],
            LinkSource::Manual,
        )
        .await
        .unwrap();
    let link = link_settled(&grabber, added.added[0].id).await;
    grabber.rename(link.id, "Renamed Album").await.unwrap();
    let packages = grabber.confirm(&[link.id], &dest, &manager).await.unwrap();
    let id = first_download(&manager, packages[0]).await;

    let download = until_completed(&manager, id).await;
    assert_eq!(download.dest_path, Some(dest.join("Renamed Album")));
    assert_eq!(
        std::fs::read(dest.join("Renamed Album").join("a.bin")).unwrap(),
        a
    );
    assert_eq!(
        std::fs::read(dest.join("Renamed Album").join("b.bin")).unwrap(),
        b
    );
    assert!(
        !dest.join("album").exists(),
        "the torrent's own name was used"
    );
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_keeps_the_chosen_files_and_the_renamed_folder() {
    let root = tempfile::tempdir().unwrap();
    let (seeder, a, _) = seed_album(root.path()).await;
    let torrent_file = write_file(&root.path().join("links"), "album.torrent", &seeder.torrent);
    let dest = root.path().join("downloads");
    std::fs::create_dir_all(&dest).unwrap();
    let state = root.path().join("state");
    let pool = open_pool(root.path()).await;

    let first = Manager::start(pool.clone(), config(&state, seeder.addr))
        .await
        .unwrap();
    let grabber = LinkGrabber::start(pool.clone(), None, None, first.torrents())
        .await
        .unwrap();
    let added = grabber
        .add(
            vec![Url::from_file_path(&torrent_file).unwrap()],
            LinkSource::Manual,
        )
        .await
        .unwrap();
    let link = link_settled(&grabber, added.added[0].id).await;
    grabber
        .set_torrent_files(&[link.id], Some(vec![0]))
        .await
        .unwrap();
    grabber.rename(link.id, "Kept").await.unwrap();
    // Slow enough to stop part-way through the chosen 1 MiB file.
    first.set_speed_limit(Some(512 * 1024));
    let packages = grabber.confirm(&[link.id], &dest, &first).await.unwrap();
    let id = first_download(&first, packages[0]).await;
    until(&first, id, "some progress", |d| d.downloaded > 0).await;
    first.shutdown().await;
    assert_eq!(first.get(id).await.unwrap().status, Status::Queued);
    drop(grabber);
    drop(first);

    let second = Manager::start(pool, config(&state, seeder.addr))
        .await
        .unwrap();
    let download = until_completed(&second, id).await;
    assert_eq!(download.dest_path, Some(dest.join("Kept")));
    assert_eq!(std::fs::read(dest.join("Kept").join("a.bin")).unwrap(), a);
    let skipped = dest.join("Kept").join("b.bin");
    assert_eq!(
        std::fs::metadata(&skipped).unwrap().len(),
        0,
        "the file that wasn't chosen was downloaded after the restart"
    );
    second.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn set_torrent_files_refuses_an_empty_or_unknown_choice() {
    let root = tempfile::tempdir().unwrap();
    let (seeder, _, _) = seed_album(root.path()).await;
    let torrent_file = write_file(&root.path().join("links"), "album.torrent", &seeder.torrent);
    let (manager, grabber) = manager_and_grabber(root.path(), seeder.addr).await;

    let added = grabber
        .add(
            vec![Url::from_file_path(&torrent_file).unwrap()],
            LinkSource::Manual,
        )
        .await
        .unwrap();
    let id = added.added[0].id;
    link_settled(&grabber, id).await;

    let empty = grabber.set_torrent_files(&[id], Some(Vec::new())).await;
    assert!(
        matches!(&empty, Err(e) if e.to_string().contains("Choose at least one file")),
        "{empty:?}"
    );
    // The album has files 0 and 1 only.
    let unknown = grabber.set_torrent_files(&[id], Some(vec![7])).await;
    assert!(unknown.is_err(), "{unknown:?}");
    assert_eq!(
        grabber.list().await.unwrap()[0].torrent_options,
        Some(TorrentOptions::default()),
        "a refused choice changed nothing"
    );
    // Dropping indices the torrent doesn't have keeps the rest.
    grabber
        .set_torrent_files(&[id], Some(vec![1, 1, 9]))
        .await
        .unwrap();
    assert_eq!(
        grabber.list().await.unwrap()[0].torrent_options,
        Some(TorrentOptions {
            only_files: Some(vec![1])
        })
    );
    // Back to all of them.
    grabber.set_torrent_files(&[id], None).await.unwrap();
    assert_eq!(
        grabber.list().await.unwrap()[0].torrent_options,
        Some(TorrentOptions::default())
    );
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_torrent_file_is_checked_and_copied_into_the_torrent_folder() {
    let root = tempfile::tempdir().unwrap();
    let (seeder, _, _) = seed_album(root.path()).await;
    let (manager, grabber) = manager_and_grabber(root.path(), seeder.addr).await;

    let outcome = grabber
        .add_torrent_bytes("album.torrent", seeder.torrent.clone(), LinkSource::Drop)
        .await
        .unwrap();
    assert_eq!(outcome.added.len(), 1);
    let link = link_settled(&grabber, outcome.added[0].id).await;
    assert_eq!(link.availability, Availability::Online, "{:?}", link.error);
    assert_eq!(link.package, "album", "named after the torrent");
    assert_eq!(link.torrent.as_ref().unwrap().files.len(), 2);

    let copy = root
        .path()
        .join("state")
        .join("files")
        .join(format!("{}.torrent", seeder.info_hash));
    assert_eq!(std::fs::read(&copy).unwrap(), seeder.torrent);

    // Adding the same torrent again is a duplicate, not a second link.
    let again = grabber
        .add_torrent_bytes("album.torrent", seeder.torrent.clone(), LinkSource::Drop)
        .await
        .unwrap();
    assert!(again.added.is_empty());
    assert_eq!(again.duplicates, 1);

    let not_torrent = grabber
        .add_torrent_bytes(
            "notes.torrent",
            b"hello, not bencode".to_vec(),
            LinkSource::Drop,
        )
        .await;
    assert!(
        matches!(&not_torrent, Err(e) if e.to_string().contains("isn't a .torrent file")),
        "{not_torrent:?}"
    );
    manager.shutdown().await;
}
