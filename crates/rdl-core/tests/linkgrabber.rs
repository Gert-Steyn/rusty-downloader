//! LinkGrabber tests: probing, availability, de-duplication, package grouping,
//! user edits winning over probes, confirming into the manager, and restart.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{SIZE, spawn_server};
use rdl_core::db::{self, Pool};
use rdl_core::linkgrabber::{Availability, GrabbedLink, LinkGrabber, LinkSource};
use rdl_core::manager::{Manager, ManagerConfig};
use reqwest::Url;

const TIMEOUT: Duration = Duration::from_secs(10);

struct Fixture {
    dir: tempfile::TempDir,
    pool: Pool,
    grabber: LinkGrabber,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(&dir.path().join("test.db")).await.unwrap();
        let grabber = LinkGrabber::start(pool.clone(), None, None).await.unwrap();
        Self { dir, pool, grabber }
    }

    async fn add(&self, base: &Url, paths: &[&str]) -> Vec<GrabbedLink> {
        let urls = paths.iter().map(|p| base.join(p).unwrap()).collect();
        self.grabber
            .add(urls, LinkSource::Manual)
            .await
            .unwrap()
            .added
    }

    /// Poll until every link has finished checking.
    async fn settled(&self) -> Vec<GrabbedLink> {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                let links = self.grabber.list().await.unwrap();
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
        .expect("links never finished checking")
    }
}

fn by_url<'a>(links: &'a [GrabbedLink], path: &str) -> &'a GrabbedLink {
    links
        .iter()
        .find(|l| l.url.ends_with(path))
        .unwrap_or_else(|| panic!("no link for {path}"))
}

#[tokio::test]
async fn probes_set_name_size_and_availability() {
    let (base, _server) = spawn_server().await;
    let fx = Fixture::new().await;
    let added = fx.add(&base, &["file", "gone", "named/report.pdf"]).await;
    assert_eq!(added.len(), 3);
    assert!(
        added
            .iter()
            .all(|l| l.availability == Availability::Unknown)
    );

    let links = fx.settled().await;

    let file = by_url(&links, "/file");
    assert_eq!(file.availability, Availability::Online);
    assert_eq!(file.filename.as_deref(), Some("test.bin"));
    assert_eq!(file.size, Some(SIZE as u64));
    assert_eq!(file.host, "127.0.0.1");

    let gone = by_url(&links, "/gone");
    assert_eq!(gone.availability, Availability::Offline);
    assert!(gone.error.as_deref().unwrap().contains("410"));

    assert_eq!(
        by_url(&links, "report.pdf").filename.as_deref(),
        Some("report.pdf")
    );
}

#[tokio::test]
async fn unreachable_hosts_are_errors_not_offline() {
    let fx = Fixture::new().await;
    // Nothing listens on port 9 (discard) locally.
    let url = Url::parse("http://127.0.0.1:9/nothing").unwrap();
    fx.grabber.add(vec![url], LinkSource::Manual).await.unwrap();

    let links = fx.settled().await;

    assert_eq!(links[0].availability, Availability::Error);
    assert!(links[0].error.is_some());
}

#[tokio::test]
async fn duplicates_are_skipped() {
    let (base, _server) = spawn_server().await;
    let fx = Fixture::new().await;
    fx.add(&base, &["file"]).await;

    let urls = vec![
        base.join("file").unwrap(),
        base.join("plain").unwrap(),
        base.join("plain").unwrap(),
    ];
    let outcome = fx.grabber.add(urls, LinkSource::Clipboard).await.unwrap();

    assert_eq!(outcome.added.len(), 1);
    assert_eq!(outcome.duplicates, 2);
    assert_eq!(fx.grabber.list().await.unwrap().len(), 2);
}

#[tokio::test]
async fn split_archives_are_grouped_by_base_name() {
    let (base, _server) = spawn_server().await;
    let fx = Fixture::new().await;
    fx.add(
        &base,
        &[
            "named/movie.part1.rar",
            "named/movie.part2.rar",
            "named/notes.txt",
        ],
    )
    .await;

    let links = fx.settled().await;

    assert_eq!(by_url(&links, "part1.rar").package, "movie");
    assert_eq!(by_url(&links, "part2.rar").package, "movie");
    assert_eq!(by_url(&links, "notes.txt").package, "127.0.0.1");
}

#[tokio::test]
async fn user_edits_survive_a_recheck() {
    let (base, _server) = spawn_server().await;
    let fx = Fixture::new().await;
    let link = fx.add(&base, &["named/movie.part1.rar"]).await.remove(0);
    fx.settled().await;

    fx.grabber.rename(link.id, "my: movie.rar").await.unwrap();
    fx.grabber.set_package(&[link.id], "Holiday").await.unwrap();
    fx.grabber.recheck(&[link.id]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let links = fx.settled().await;

    assert_eq!(
        links[0].filename.as_deref(),
        Some("my_ movie.rar"),
        "name is sanitised and kept"
    );
    assert_eq!(links[0].package, "Holiday");
    assert!(fx.grabber.set_package(&[link.id], "  ").await.is_err());
}

#[tokio::test]
async fn confirm_creates_one_package_per_name_and_folder() {
    let (base, _server) = spawn_server().await;
    let fx = Fixture::new().await;
    let links = fx
        .add(
            &base,
            &[
                "named/movie.part1.rar",
                "named/movie.part2.rar",
                "named/notes.txt",
            ],
        )
        .await;
    fx.settled().await;
    let notes = links.iter().find(|l| l.url.ends_with("notes.txt")).unwrap();
    let elsewhere = fx.dir.path().join("elsewhere");
    fx.grabber
        .set_dest_dir(&[notes.id], Some(&elsewhere))
        .await
        .unwrap();

    let manager = Manager::start(fx.pool.clone(), ManagerConfig::default())
        .await
        .unwrap();
    let default_dest: PathBuf = fx.dir.path().join("downloads");
    let ids: Vec<_> = links.iter().map(|l| l.id).collect();
    let created = fx
        .grabber
        .confirm(&ids, &default_dest, &manager)
        .await
        .unwrap();

    assert_eq!(created.len(), 2);
    let packages = manager.list().await.unwrap();
    let movie = packages.iter().find(|p| p.name == "movie").unwrap();
    assert_eq!(movie.dest_dir, default_dest);
    let names: Vec<_> = movie
        .downloads
        .iter()
        .map(|d| d.filename.clone().unwrap())
        .collect();
    assert_eq!(names, ["movie.part1.rar", "movie.part2.rar"]);
    let host = packages.iter().find(|p| p.name == "127.0.0.1").unwrap();
    assert_eq!(host.dest_dir, elsewhere);
    assert!(
        fx.grabber.list().await.unwrap().is_empty(),
        "confirmed links leave the grabber"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn unfinished_checks_resume_after_restart() {
    let (base, _server) = spawn_server().await;
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&dir.path().join("test.db")).await.unwrap();
    sqlx::query("INSERT INTO link_grabber (source_url, kind, availability, proposed_package) VALUES (?, 'http', 'checking', 'p')")
        .bind(base.join("file").unwrap().as_str())
        .execute(&pool)
        .await
        .unwrap();

    let grabber = LinkGrabber::start(pool.clone(), None, None).await.unwrap();
    let fx = Fixture { dir, pool, grabber };
    let links = fx.settled().await;

    assert_eq!(links[0].availability, Availability::Online);
}
