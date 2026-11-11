//! FTP downloads against a local server: probing, logins from the link or
//! an account, `REST` resume after a dropped connection and after a pause
//! (in the engine and through the manager), a server that can't resume, and
//! links through LinkGrabber and the manager.

mod common;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::ftp::{FtpServer, PASSWORD, PATH, USER, spawn_ftp};
use rdl_core::Error;
use rdl_core::accounts::{AccountInput, Login};
use rdl_core::db;
use rdl_core::ftp::{self, Location};
use rdl_core::http::{Outcome, Progress, SegmentConfig};
use rdl_core::linkgrabber::{Availability, LinkGrabber, LinkSource};
use rdl_core::manager::{
    DEFAULT_QUEUE, DownloadKind, Event, Manager, ManagerConfig, NewDownload, NewPackage, Status,
};
use rdl_core::network::Network;
use rdl_core::secrets::MemorySecrets;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::Url;

const SIZE: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

struct Run {
    dir: tempfile::TempDir,
    config: SegmentConfig,
}

impl Run {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            config: SegmentConfig {
                max_retries: 3,
                retry_base_delay: Duration::from_millis(10),
                progress_interval: Duration::from_millis(10),
                ..SegmentConfig::default()
            },
        }
    }

    fn part(&self) -> PathBuf {
        self.dir.path().join("file.bin.rdl.part")
    }

    fn dest(&self) -> PathBuf {
        self.dir.path().join("out").join("file.bin")
    }

    /// Probe and download `url`, resuming from the part file if `resume`.
    async fn download(
        &self,
        url: &Url,
        login: Option<Login>,
        resume: bool,
        cancel: &CancellationToken,
        progress: &watch::Sender<Progress>,
    ) -> rdl_core::Result<Outcome> {
        let location = Location::new(url, login, None)?;
        let probe = ftp::probe(&location).await?;
        let (part, dest) = (self.part(), self.dest());
        let run = ftp::download(
            &location,
            &probe,
            &part,
            &dest,
            resume,
            &self.config,
            progress,
            cancel,
        );
        tokio::time::timeout(TIMEOUT, run)
            .await
            .expect("download hung")
    }

    async fn fetch(&self, url: &Url, login: Option<Login>) -> rdl_core::Result<Outcome> {
        let (tx, _rx) = watch::channel(Progress::default());
        self.download(url, login, false, &CancellationToken::new(), &tx)
            .await
    }

    fn assert_complete(&self, server: &FtpServer) {
        assert_eq!(std::fs::read(self.dest()).unwrap(), server.data);
        assert!(!self.part().exists());
    }
}

#[tokio::test]
async fn probes_and_downloads_anonymously() {
    let server = spawn_ftp(SIZE).await;
    let url = server.url(None, PATH);
    let probe = ftp::probe(&Location::new(&url, None, None).unwrap())
        .await
        .unwrap();
    assert_eq!(probe.size, Some(SIZE as u64));
    assert_eq!(probe.filename, "file.bin");
    assert_eq!(probe.last_modified.as_deref(), Some("20261101120000"));

    let run = Run::new();
    let outcome = run.fetch(&url, None).await.unwrap();
    assert_eq!(
        outcome,
        Outcome::Completed {
            path: run.dest(),
            size: SIZE as u64
        }
    );
    run.assert_complete(&server);
    assert!(server.logins().iter().all(|u| u == "anonymous"));

    let missing =
        ftp::probe(&Location::new(&server.url(None, "pub/nope.bin"), None, None).unwrap())
            .await
            .unwrap_err();
    assert!(
        matches!(missing, Error::FtpReply { code: 550, .. }),
        "{missing}"
    );
}

#[tokio::test]
async fn logins_come_from_the_link_or_the_account() {
    let server = spawn_ftp(SIZE).await;
    server.anonymous.store(false, Ordering::SeqCst);
    let account = Login {
        site: "127.0.0.1".into(),
        username: USER.into(),
        password: PASSWORD.into(),
    };

    let refused = Run::new()
        .fetch(&server.url(None, PATH), None)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, Error::FtpReply { code: 530, .. }),
        "{refused}"
    );
    assert!(!refused.is_retryable());

    let run = Run::new();
    run.fetch(&server.url(Some((USER, PASSWORD)), PATH), None)
        .await
        .unwrap();
    run.assert_complete(&server);

    let run = Run::new();
    run.fetch(&server.url(None, PATH), Some(account.clone()))
        .await
        .unwrap();
    run.assert_complete(&server);

    // The link's own login wins over the account's.
    let wrong = Run::new()
        .fetch(&server.url(Some((USER, "nope")), PATH), Some(account))
        .await
        .unwrap_err();
    assert!(
        matches!(wrong, Error::FtpReply { code: 530, .. }),
        "{wrong}"
    );
    assert!(server.logins().iter().all(|u| u == USER));
}

#[tokio::test]
async fn a_dropped_transfer_resumes_where_it_stopped() {
    let server = spawn_ftp(SIZE).await;
    server.drops_left.store(1, Ordering::SeqCst);
    let run = Run::new();
    run.fetch(&server.url(None, PATH), None).await.unwrap();
    run.assert_complete(&server);
    let rests = server.rests();
    assert_eq!(rests.len(), 1, "{rests:?}");
    assert!(rests[0] > 0 && rests[0] < SIZE as u64);
    // Nothing was fetched twice.
    assert_eq!(server.bytes_served.load(Ordering::SeqCst), SIZE as u64);
}

#[tokio::test]
async fn pausing_keeps_the_part_file_and_resuming_continues_it() {
    let server = spawn_ftp(SIZE).await;
    server.slow.store(true, Ordering::SeqCst);
    let url = server.url(None, PATH);
    let run = Run::new();
    let cancel = CancellationToken::new();
    let (tx, mut rx) = watch::channel(Progress::default());
    let stopper = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            rx.wait_for(|p| p.downloaded >= 64 * 1024).await.unwrap();
            cancel.cancel();
        })
    };
    let outcome = run.download(&url, None, false, &cancel, &tx).await.unwrap();
    stopper.await.unwrap();
    assert_eq!(outcome, Outcome::Paused);
    let kept = std::fs::metadata(run.part()).unwrap().len();
    assert!(kept > 0 && kept < SIZE as u64);
    let last = tx.borrow().clone();
    assert_eq!(last.downloaded, kept);
    assert_eq!(last.segments.len(), 1);

    server.slow.store(false, Ordering::SeqCst);
    let (tx, _rx) = watch::channel(Progress::default());
    run.download(&url, None, true, &CancellationToken::new(), &tx)
        .await
        .unwrap();
    run.assert_complete(&server);
    assert_eq!(server.rests(), [kept]);
}

#[tokio::test]
async fn a_server_that_cannot_resume_starts_over() {
    let server = spawn_ftp(SIZE).await;
    server.no_rest.store(true, Ordering::SeqCst);
    let run = Run::new();
    std::fs::write(run.part(), &server.data[..1000]).unwrap();
    let (tx, _rx) = watch::channel(Progress::default());
    run.download(
        &server.url(None, PATH),
        None,
        true,
        &CancellationToken::new(),
        &tx,
    )
    .await
    .unwrap();
    run.assert_complete(&server);
    assert_eq!(server.bytes_served.load(Ordering::SeqCst), SIZE as u64);
}

#[tokio::test]
async fn ftp_links_go_through_linkgrabber_and_the_manager() {
    let server = spawn_ftp(SIZE).await;
    server.anonymous.store(false, Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&dir.path().join("test.db")).await.unwrap();
    let network = Network::load(pool.clone(), MemorySecrets::new(), None)
        .await
        .unwrap();
    network
        .add_account(AccountInput {
            site: "127.0.0.1".into(),
            username: Some(USER.into()),
            password: Some(PASSWORD.into()),
            enabled: true,
        })
        .await
        .unwrap();

    let grabber = LinkGrabber::start(pool.clone(), Some(network.clone()), None, None)
        .await
        .unwrap();
    let url = server.url(None, PATH);
    grabber
        .add(
            vec![url.clone(), server.url(None, "pub/nope.bin")],
            LinkSource::Manual,
        )
        .await
        .unwrap();
    let links = tokio::time::timeout(TIMEOUT, async {
        loop {
            let links = grabber.list().await.unwrap();
            if links.iter().all(|l| {
                !matches!(
                    l.availability,
                    Availability::Unknown | Availability::Checking
                )
            }) {
                return links;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("links never finished checking");
    let found = links.iter().find(|l| l.url.ends_with(PATH)).unwrap();
    assert_eq!(
        found.availability,
        Availability::Online,
        "{:?}",
        found.error
    );
    assert_eq!(found.kind, DownloadKind::Ftp);
    assert_eq!(found.size, Some(SIZE as u64));
    assert_eq!(found.filename.as_deref(), Some("file.bin"));
    let missing = links.iter().find(|l| l.url.ends_with("nope.bin")).unwrap();
    assert_eq!(missing.availability, Availability::Offline);

    let manager = Manager::start(
        pool.clone(),
        ManagerConfig {
            network: Some(network.clone()),
            ..ManagerConfig::default()
        },
    )
    .await
    .unwrap();
    let mut events = manager.subscribe();
    let download = NewDownload::new(url);
    assert_eq!(download.kind, DownloadKind::Ftp);
    server.slow.store(true, Ordering::SeqCst);
    manager
        .add_package(NewPackage {
            name: "p".into(),
            dest_dir: dir.path().join("downloads"),
            queue_id: DEFAULT_QUEUE,
            downloads: vec![download],
        })
        .await
        .unwrap();
    let id = manager.list().await.unwrap()[0].downloads[0].id;

    // Paused part-way, it resumes with `REST` from what the part file holds.
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let Ok(Event::Progress { downloaded, .. }) = events.recv().await
                && downloaded >= 64 * 1024
            {
                return;
            }
        }
    })
    .await
    .expect("no progress");
    manager.pause(id).await.unwrap();
    tokio::time::timeout(TIMEOUT, async {
        while manager.get(id).await.unwrap().status != Status::Paused {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("never paused");
    let paused = manager.get(id).await.unwrap();
    let mut part = paused.dest_path.clone().unwrap().into_os_string();
    part.push(".rdl.part");
    let kept = std::fs::metadata(&part).unwrap().len();
    assert!(kept > 0 && kept < SIZE as u64);
    assert_eq!(paused.downloaded, kept);
    server.slow.store(false, Ordering::SeqCst);
    manager.resume(id).await.unwrap();

    let status = tokio::time::timeout(TIMEOUT, async {
        loop {
            if let Ok(Event::StatusChanged {
                status: status @ (Status::Completed | Status::Failed),
                error,
                ..
            }) = events.recv().await
            {
                return (status, error);
            }
        }
    })
    .await
    .expect("download didn't finish");
    assert_eq!(status, (Status::Completed, None));
    let stored = &manager.list().await.unwrap()[0].downloads[0];
    assert_eq!(stored.kind, "ftp");
    assert_eq!(stored.size, Some(SIZE as u64));
    let path = stored.dest_path.clone().unwrap();
    assert!(path.ends_with("file.bin"));
    assert_eq!(std::fs::read(path).unwrap(), server.data);
    assert_eq!(server.rests(), [kept]);
}
