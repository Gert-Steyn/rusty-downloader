//! Proxies end to end: profiles and their passwords, rules, downloads and
//! link checks through local SOCKS5 and HTTP proxies, and the test button.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{SIZE, proxy, spawn_server};
use rdl_core::db::{self, Pool};
use rdl_core::linkgrabber::{Availability, LinkGrabber, LinkSource};
use rdl_core::manager::{
    DEFAULT_QUEUE, DownloadId, Event, Manager, ManagerConfig, NewDownload, NewPackage, Status,
};
use rdl_core::network::{Binding, Network, ToolConnection};
use rdl_core::proxy::{ProxyChoice, ProxyInput, ProxyKind, ProxyRule, ProxySettings};
use rdl_core::secrets::{MemorySecrets, Secrets};
use reqwest::Url;

const TIMEOUT: Duration = Duration::from_secs(15);

struct Fixture {
    dir: tempfile::TempDir,
    pool: Pool,
    secrets: Arc<MemorySecrets>,
    network: Network,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(&dir.path().join("test.db")).await.unwrap();
        let secrets = MemorySecrets::new();
        let network = Network::load(pool.clone(), secrets.clone(), None)
            .await
            .unwrap();
        Self {
            dir,
            pool,
            secrets,
            network,
        }
    }

    async fn manager(&self) -> Manager {
        Manager::start(
            self.pool.clone(),
            ManagerConfig {
                network: Some(self.network.clone()),
                ..ManagerConfig::default()
            },
        )
        .await
        .unwrap()
    }

    /// Download `url` with `proxy` and wait until it completes or fails.
    async fn download(
        &self,
        manager: &Manager,
        url: Url,
        proxy: ProxyChoice,
    ) -> (DownloadId, Status) {
        let mut events = manager.subscribe();
        let package = manager
            .add_package(NewPackage {
                name: "p".into(),
                dest_dir: self.dir.path().join("downloads"),
                queue_id: DEFAULT_QUEUE,
                downloads: vec![NewDownload {
                    proxy,
                    ..NewDownload::new(url)
                }],
            })
            .await
            .unwrap();
        let packages = manager.list().await.unwrap();
        let id = packages.iter().find(|p| p.id == package).unwrap().downloads[0].id;
        let status = tokio::time::timeout(TIMEOUT, async {
            loop {
                if let Ok(Event::StatusChanged {
                    download_id,
                    status: status @ (Status::Completed | Status::Failed),
                    ..
                }) = events.recv().await
                    && download_id == id
                {
                    return status;
                }
            }
        })
        .await
        .expect("download didn't finish");
        (id, status)
    }
}

fn input(name: &str, kind: ProxyKind, port: u16, login: Option<(&str, &str)>) -> ProxyInput {
    ProxyInput {
        name: name.into(),
        kind,
        host: "127.0.0.1".into(),
        port,
        username: login.map(|(u, _)| u.into()),
        password: login.map(|(_, p)| p.into()),
    }
}

#[tokio::test]
async fn profiles_keep_passwords_out_of_the_database() {
    let fx = Fixture::new().await;
    let net = &fx.network;
    let id = net
        .add_profile(input(
            "Home",
            ProxyKind::Socks5,
            1080,
            Some(("me", "s3cret")),
        ))
        .await
        .unwrap();
    let p = &net.profiles()[0];
    assert!(p.has_password);
    assert_eq!(p.username.as_deref(), Some("me"));
    assert_eq!(
        fx.secrets.get(&format!("proxy/{id}")).unwrap().as_deref(),
        Some("s3cret")
    );
    let row: String = sqlx::query_scalar(
        "SELECT json_array(name, kind, host, port, username, keyring_ref) FROM proxy_profiles",
    )
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert!(!row.contains("s3cret"), "password in the database: {row}");

    assert!(
        net.add_profile(input("Home", ProxyKind::Http, 8080, None))
            .await
            .unwrap_err()
            .to_string()
            .contains("already a proxy called")
    );

    // No password given keeps it; an empty one removes it.
    let mut change = input("Home", ProxyKind::Socks5, 1081, Some(("me", "x")));
    change.password = None;
    net.update_profile(id, change.clone()).await.unwrap();
    assert!(net.profiles()[0].has_password);
    assert_eq!(net.profiles()[0].port, 1081);
    change.password = Some(String::new());
    net.update_profile(id, change).await.unwrap();
    assert!(!net.profiles()[0].has_password);
    assert_eq!(fx.secrets.get(&format!("proxy/{id}")).unwrap(), None);

    // Removing it takes its rules and the default with it.
    let other = net
        .add_profile(input("Other", ProxyKind::Http, 3128, None))
        .await
        .unwrap();
    net.set_settings(ProxySettings {
        default: Some(id),
        rules: vec![
            ProxyRule {
                pattern: "a.test".into(),
                profile: Some(id),
            },
            ProxyRule {
                pattern: "b.test".into(),
                profile: Some(other),
            },
        ],
    })
    .await
    .unwrap();
    net.remove_profile(id).await.unwrap();
    assert_eq!(
        net.settings(),
        ProxySettings {
            default: None,
            rules: vec![ProxyRule {
                pattern: "b.test".into(),
                profile: Some(other)
            }],
        }
    );
    // And they survive a restart.
    let again = Network::load(fx.pool.clone(), fx.secrets.clone(), None)
        .await
        .unwrap();
    assert_eq!(again.settings(), net.settings());
    assert_eq!(again.profiles(), net.profiles());
}

#[tokio::test]
async fn downloads_go_through_socks5_by_default_rule_or_choice() {
    let (base, server) = spawn_server().await;
    let socks = proxy::socks5(Some(("me", "s3cret"))).await;
    let fx = Fixture::new().await;
    let good = fx
        .network
        .add_profile(input(
            "Good",
            ProxyKind::Socks5,
            socks.port,
            Some(("me", "s3cret")),
        ))
        .await
        .unwrap();
    let bad = fx
        .network
        .add_profile(input(
            "Bad",
            ProxyKind::Socks5,
            socks.port,
            Some(("me", "wrong")),
        ))
        .await
        .unwrap();
    fx.network
        .set_settings(ProxySettings {
            default: Some(good),
            rules: vec![],
        })
        .await
        .unwrap();
    let manager = fx.manager().await;
    let file = base.join("named/one.bin").unwrap();

    // The default proxy carries it.
    let (id, status) = fx.download(&manager, file.clone(), ProxyChoice::Auto).await;
    assert_eq!(status, Status::Completed);
    assert!(socks.tunnels() > 0, "didn't go through the proxy");
    common::assert_matches_server(
        manager.get(id).await.unwrap().dest_path.as_ref().unwrap(),
        &server,
    );

    // A rule sends this host directly.
    fx.network
        .set_settings(ProxySettings {
            default: Some(good),
            rules: vec![ProxyRule {
                pattern: "127.0.0.1".into(),
                profile: None,
            }],
        })
        .await
        .unwrap();
    let before = socks.tunnels();
    let (_, status) = fx
        .download(
            &manager,
            base.join("named/two.bin").unwrap(),
            ProxyChoice::Auto,
        )
        .await;
    assert_eq!(status, Status::Completed);
    assert_eq!(socks.tunnels(), before, "the direct rule was ignored");

    // A download's own choice beats the rule; a wrong login fails it.
    let (_, status) = fx
        .download(
            &manager,
            base.join("named/three.bin").unwrap(),
            ProxyChoice::Profile { id: good },
        )
        .await;
    assert_eq!(status, Status::Completed);
    assert!(socks.tunnels() > before);
    let (id, status) = fx
        .download(
            &manager,
            base.join("named/four.bin").unwrap(),
            ProxyChoice::Profile { id: bad },
        )
        .await;
    assert_eq!(status, Status::Failed);
    assert!(socks.refused() > 0);
    assert_eq!(
        manager.get(id).await.unwrap().proxy,
        ProxyChoice::Profile { id: bad }
    );

    // Changing the choice afterwards is stored and announced.
    let mut events = manager.subscribe();
    manager
        .set_download_proxy(id, ProxyChoice::Direct)
        .await
        .unwrap();
    assert_eq!(manager.get(id).await.unwrap().proxy, ProxyChoice::Direct);
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ProxyChanged {
            proxy: ProxyChoice::Direct,
            ..
        }
    ));
    manager.shutdown().await;
    assert_eq!(SIZE, server.data.len());
}

#[tokio::test]
async fn http_proxy_link_checks_and_the_test_button() {
    let (base, _server) = spawn_server().await;
    let http = proxy::http(Some(("me", "pa:ss"))).await;
    let fx = Fixture::new().await;
    let good = fx
        .network
        .add_profile(input(
            "Office",
            ProxyKind::Http,
            http.port,
            Some(("me", "pa:ss")),
        ))
        .await
        .unwrap();
    let bad = fx
        .network
        .add_profile(input(
            "Wrong",
            ProxyKind::Http,
            http.port,
            Some(("me", "nope")),
        ))
        .await
        .unwrap();
    let file = base.join("file").unwrap();

    fx.network
        .test(&file, ProxyChoice::Profile { id: good })
        .await
        .unwrap();
    assert!(http.tunnels() > 0);
    let err = fx
        .network
        .test(&file, ProxyChoice::Profile { id: bad })
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("407"), "{err}");
    fx.network.test(&file, ProxyChoice::Direct).await.unwrap();

    // Link checks follow the rules too.
    fx.network
        .set_settings(ProxySettings {
            default: None,
            rules: vec![ProxyRule {
                pattern: "127.0.0.1".into(),
                profile: Some(good),
            }],
        })
        .await
        .unwrap();
    let before = http.tunnels();
    let grabber = LinkGrabber::start(fx.pool.clone(), Some(fx.network.clone()), None)
        .await
        .unwrap();
    let mut events = grabber.subscribe();
    grabber
        .add(vec![file.clone()], LinkSource::Manual)
        .await
        .unwrap();
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let _ = events.recv().await;
            let links = grabber.list().await.unwrap();
            if links.iter().all(|l| l.availability == Availability::Online) {
                break;
            }
        }
    })
    .await
    .expect("link never came online");
    assert!(
        http.tunnels() > before,
        "the link check went around the proxy"
    );
}

#[tokio::test]
async fn binding_sends_from_the_address_or_not_at_all() {
    let (base, server) = spawn_server().await;
    let fx = Fixture::new().await;
    let manager = fx.manager().await;

    // Bound to loopback, the local server is still reachable.
    fx.network
        .set_binding(Binding::Bound("127.0.0.1".parse().unwrap()));
    let (id, status) = fx
        .download(
            &manager,
            base.join("named/bound.bin").unwrap(),
            ProxyChoice::Auto,
        )
        .await;
    assert_eq!(status, Status::Completed);
    common::assert_matches_server(
        manager.get(id).await.unwrap().dest_path.as_ref().unwrap(),
        &server,
    );

    // An address this machine doesn't have can't be sent from.
    fx.network
        .set_binding(Binding::Bound("192.0.2.1".parse().unwrap()));
    let (_, status) = fx
        .download(
            &manager,
            base.join("named/nowhere.bin").unwrap(),
            ProxyChoice::Auto,
        )
        .await;
    assert_eq!(status, Status::Failed);

    // Down: nothing connects, and the tools are refused too.
    fx.network.set_binding(Binding::Down("VPN".into()));
    let (id, status) = fx
        .download(
            &manager,
            base.join("named/down.bin").unwrap(),
            ProxyChoice::Auto,
        )
        .await;
    assert_eq!(status, Status::Failed);
    let error = manager.get(id).await.unwrap().error.unwrap();
    assert!(error.contains("(VPN) is down"), "{error}");
    assert!(
        fx.network
            .tool_connection(&base, ProxyChoice::Auto)
            .is_err()
    );
    assert!(
        !fx.network.set_binding(Binding::Down("VPN".into())),
        "no change"
    );

    fx.network.set_binding(Binding::Any);
    assert_eq!(
        fx.network
            .tool_connection(&base, ProxyChoice::Auto)
            .unwrap(),
        ToolConnection::default()
    );
    manager.shutdown().await;
}

/// Poll until download `id` has `status`.
async fn status_becomes(manager: &Manager, id: DownloadId, status: Status) {
    tokio::time::timeout(TIMEOUT, async {
        while manager.get(id).await.unwrap().status != status {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("download never became {status:?}"));
}

#[tokio::test]
async fn a_network_hold_requeues_and_resumes_downloads() {
    let (base, server) = spawn_server().await;
    let fx = Fixture::new().await;
    let manager = fx.manager().await;
    let package = manager
        .add_package(NewPackage {
            name: "p".into(),
            dest_dir: fx.dir.path().join("downloads"),
            queue_id: DEFAULT_QUEUE,
            downloads: vec![NewDownload::new(base.join("slow").unwrap())],
        })
        .await
        .unwrap();
    let id = manager
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.id == package)
        .unwrap()
        .downloads[0]
        .id;
    status_becomes(&manager, id, Status::Active).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    manager.set_network_hold(Some("VPN is down".into()));
    status_becomes(&manager, id, Status::Queued).await;
    let kept = manager.get(id).await.unwrap().downloaded;
    assert!(kept > 0, "progress wasn't kept");
    assert_eq!(manager.network_hold().as_deref(), Some("VPN is down"));
    // Held: it doesn't start again.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(manager.get(id).await.unwrap().status, Status::Queued);
    assert_eq!(manager.running_count(), 0);

    // Released: it carries on from where it was.
    let served = server.served();
    manager.set_network_hold(None);
    status_becomes(&manager, id, Status::Completed).await;
    assert!(
        server.served() - served < SIZE as u64,
        "started over instead of resuming"
    );

    // A restart (new address) requeues what runs, and it finishes anyway.
    let package = manager
        .add_package(NewPackage {
            name: "q".into(),
            dest_dir: fx.dir.path().join("downloads"),
            queue_id: DEFAULT_QUEUE,
            downloads: vec![NewDownload::new(base.join("slow").unwrap())],
        })
        .await
        .unwrap();
    let id = manager
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.id == package)
        .unwrap()
        .downloads[0]
        .id;
    status_becomes(&manager, id, Status::Active).await;
    let mut events = manager.subscribe();
    manager.restart_running();
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let Ok(Event::StatusChanged {
                download_id,
                status: Status::Queued,
                ..
            }) = events.recv().await
                && download_id == id
            {
                break;
            }
        }
    })
    .await
    .expect("wasn't requeued");
    status_becomes(&manager, id, Status::Completed).await;
    manager.shutdown().await;
}
