//! SFTP downloads against a local server: probing, password and key logins,
//! host keys trusted on first use and refused when they change, resuming
//! after a dropped transfer and after a pause, and links through LinkGrabber
//! and the manager.

mod common;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::sftp::{HOME_FILE, MTIME, PASSWORD, PATH, USER, USER_KEY, spawn_sftp};
use rdl_core::Error;
use rdl_core::accounts::AccountInput;
use rdl_core::db;
use rdl_core::http::{Outcome, Progress, SegmentConfig};
use rdl_core::linkgrabber::{Availability, LinkGrabber, LinkSource};
use rdl_core::manager::{
    DEFAULT_QUEUE, DownloadKind, Event, Manager, ManagerConfig, NewDownload, NewPackage, Status,
};
use rdl_core::network::Network;
use rdl_core::secrets::MemorySecrets;
use rdl_core::sftp::{self, KnownHosts, Location};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::Url;

const SIZE: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);

struct Run {
    dir: tempfile::TempDir,
    known: KnownHosts,
    /// Where SSH keys are looked for: none unless a test puts one there.
    keys: PathBuf,
    config: SegmentConfig,
}

impl Run {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("ssh");
        std::fs::create_dir_all(&keys).unwrap();
        Self {
            dir,
            known: KnownHosts::new(None),
            keys,
            config: SegmentConfig {
                max_retries: 3,
                retry_base_delay: Duration::from_millis(10),
                progress_interval: Duration::from_millis(10),
                ..SegmentConfig::default()
            },
        }
    }

    fn with_user_key(self) -> Self {
        std::fs::write(self.keys.join("id_ed25519"), USER_KEY).unwrap();
        self
    }

    fn part(&self) -> PathBuf {
        self.dir.path().join("file.bin.rdl.part")
    }

    fn dest(&self) -> PathBuf {
        self.dir.path().join("out").join("file.bin")
    }

    fn location(&self, url: &Url) -> rdl_core::Result<Location> {
        Ok(Location::new(url, None, None, self.known.clone())?
            .with_key_dir(Some(self.keys.clone())))
    }

    async fn download(
        &self,
        url: &Url,
        resume: bool,
        cancel: &CancellationToken,
        progress: &watch::Sender<Progress>,
    ) -> rdl_core::Result<Outcome> {
        let location = self.location(url)?;
        let probe = sftp::probe(&location).await?;
        let (part, dest) = (self.part(), self.dest());
        let run = sftp::download(
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

    async fn fetch(&self, url: &Url) -> rdl_core::Result<Outcome> {
        let (tx, _rx) = watch::channel(Progress::default());
        self.download(url, false, &CancellationToken::new(), &tx)
            .await
    }

    fn assert_complete(&self, data: &[u8]) {
        assert_eq!(std::fs::read(self.dest()).unwrap(), data);
        assert!(!self.part().exists());
    }
}

#[tokio::test]
async fn probes_and_downloads_with_a_password() {
    let server = spawn_sftp(SIZE).await;
    let run = Run::new();
    let url = server.url(USER, Some(PASSWORD), PATH);
    let probe = sftp::probe(&run.location(&url).unwrap()).await.unwrap();
    assert_eq!(probe.size, Some(SIZE as u64));
    assert_eq!(probe.filename, "file.bin");
    assert_eq!(probe.last_modified, Some(MTIME.to_string()));

    let outcome = run.fetch(&url).await.unwrap();
    assert_eq!(
        outcome,
        Outcome::Completed {
            path: run.dest(),
            size: SIZE as u64
        }
    );
    run.assert_complete(&server.data);
    assert!(
        server
            .logins()
            .iter()
            .all(|l| *l == (USER.into(), "password"))
    );

    // The host key was trusted on first use.
    let known = run.known.list().await.unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(
        (known[0].host.as_str(), known[0].port),
        ("127.0.0.1", server.port)
    );
    assert_eq!(known[0].algorithm, "ssh-ed25519");
    assert!(known[0].fingerprint.starts_with("SHA256:"));

    let missing = sftp::probe(
        &run.location(&server.url(USER, Some(PASSWORD), "/srv/nope.bin"))
            .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(missing, Error::SftpStatus { code: 2, .. }),
        "{missing}"
    );
    let folder = sftp::probe(
        &run.location(&server.url(USER, Some(PASSWORD), "/srv"))
            .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(matches!(folder, Error::InvalidUrl(_)), "{folder}");
}

#[tokio::test]
async fn keys_log_in_when_there_is_no_password() {
    let server = spawn_sftp(SIZE).await;

    let refused = Run::new()
        .fetch(&server.url(USER, None, PATH))
        .await
        .unwrap_err();
    assert!(matches!(refused, Error::SshLogin { .. }), "{refused}");
    assert!(!refused.is_retryable());
    let wrong = Run::new()
        .fetch(&server.url(USER, Some("nope"), PATH))
        .await
        .unwrap_err();
    assert!(matches!(wrong, Error::SshLogin { .. }), "{wrong}");

    // With a key, and a path in the home folder.
    let run = Run::new().with_user_key();
    run.fetch(&server.url(USER, None, &format!("/~/{HOME_FILE}")))
        .await
        .unwrap();
    run.assert_complete(&server.data);
    // A wrong password falls back to the key.
    let run = Run::new().with_user_key();
    run.fetch(&server.url(USER, Some("nope"), PATH))
        .await
        .unwrap();
    run.assert_complete(&server.data);
    assert!(server.logins().iter().all(|l| *l == (USER.into(), "key")));
}

#[tokio::test]
async fn a_changed_host_key_is_refused_until_forgotten() {
    let server = spawn_sftp(SIZE).await;
    let run = Run::new();
    let url = server.url(USER, Some(PASSWORD), PATH);
    sftp::probe(&run.location(&url).unwrap()).await.unwrap();
    let first = run.known.list().await.unwrap()[0].fingerprint.clone();

    server.host_key.store(1, Ordering::SeqCst);
    let changed = sftp::probe(&run.location(&url).unwrap()).await.unwrap_err();
    let Error::HostKeyChanged { known, offered, .. } = &changed else {
        panic!("expected a changed host key, got {changed}");
    };
    assert_eq!(known, &first);
    assert_ne!(offered, &first);
    assert!(!changed.is_retryable());
    assert!(
        server.logins().len() == 1,
        "logged in to the changed server"
    );

    run.known.forget("127.0.0.1", server.port).await.unwrap();
    sftp::probe(&run.location(&url).unwrap()).await.unwrap();
    assert_ne!(run.known.list().await.unwrap()[0].fingerprint, first);
}

#[tokio::test]
async fn a_dropped_transfer_resumes_where_it_stopped() {
    let server = spawn_sftp(SIZE).await;
    server.drops_left.store(1, Ordering::SeqCst);
    let run = Run::new();
    run.fetch(&server.url(USER, Some(PASSWORD), PATH))
        .await
        .unwrap();
    run.assert_complete(&server.data);
    let starts = server.starts();
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert_eq!(starts[0], 0);
    assert!(starts[1] > 0 && starts[1] < SIZE as u64, "{starts:?}");
}

#[tokio::test]
async fn pausing_keeps_the_part_file_and_resuming_continues_it() {
    let server = spawn_sftp(SIZE).await;
    server.slow.store(true, Ordering::SeqCst);
    let url = server.url(USER, Some(PASSWORD), PATH);
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
    let outcome = run.download(&url, false, &cancel, &tx).await.unwrap();
    stopper.await.unwrap();
    assert_eq!(outcome, Outcome::Paused);
    let kept = std::fs::metadata(run.part()).unwrap().len();
    assert!(kept > 0 && kept < SIZE as u64);
    assert_eq!(tx.borrow().downloaded, kept);

    server.slow.store(false, Ordering::SeqCst);
    let (tx, _rx) = watch::channel(Progress::default());
    run.download(&url, true, &CancellationToken::new(), &tx)
        .await
        .unwrap();
    run.assert_complete(&server.data);
    assert_eq!(server.starts().last(), Some(&kept));
}

#[tokio::test]
async fn sftp_links_go_through_linkgrabber_and_the_manager() {
    let server = spawn_sftp(SIZE).await;
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

    // No user in the links: the account's login is used.
    let base = format!("sftp://127.0.0.1:{}", server.port);
    let url = Url::parse(&format!("{base}{PATH}")).unwrap();
    let grabber = LinkGrabber::start(pool.clone(), Some(network.clone()), None, None)
        .await
        .unwrap();
    grabber
        .add(
            vec![
                url.clone(),
                Url::parse(&format!("{base}/srv/nope.bin")).unwrap(),
            ],
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
    assert_eq!(found.kind, DownloadKind::Sftp);
    assert_eq!(found.size, Some(SIZE as u64));
    assert_eq!(found.filename.as_deref(), Some("file.bin"));
    let missing = links.iter().find(|l| l.url.ends_with("nope.bin")).unwrap();
    assert_eq!(missing.availability, Availability::Offline);
    // The host key is stored in the database.
    let known = network.known_hosts().list().await.unwrap();
    assert_eq!(known.len(), 1);

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
    assert_eq!(download.kind, DownloadKind::Sftp);
    manager
        .add_package(NewPackage {
            name: "p".into(),
            dest_dir: dir.path().join("downloads"),
            queue_id: DEFAULT_QUEUE,
            downloads: vec![download],
        })
        .await
        .unwrap();
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
    assert_eq!(stored.kind, "sftp");
    let path = stored.dest_path.clone().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), server.data);
    assert!(
        server
            .logins()
            .iter()
            .all(|l| *l == (USER.into(), "password"))
    );
}
