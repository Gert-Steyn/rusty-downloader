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

#[tokio::test]
async fn links_downloaded_before_are_flagged() {
    use rdl_core::history::{self, Finished};

    let (base, _server) = spawn_server().await;
    let fx = Fixture::new().await;
    let kept = fx.dir.path().join("kept.bin");
    std::fs::write(&kept, "x").unwrap();
    let url = base.join("file#ignored-fragment").unwrap();
    history::record(
        &fx.pool,
        &Finished {
            url: url.as_str(),
            path: &kept,
            size: Some(1),
            kind: "http",
            package: None,
        },
    )
    .await
    .unwrap();

    let added = fx.add(&base, &["file", "named/new.bin"]).await;
    let previous = by_url(&added, "file")
        .previous
        .clone()
        .expect("flagged when added");
    assert!(previous.exists);
    assert_eq!(previous.path, kept);
    assert!(by_url(&added, "new.bin").previous.is_none());
    // Still flagged after the check finishes.
    assert!(by_url(&fx.settled().await, "file").previous.is_some());

    fx.grabber.set_warn_duplicates(false);
    assert!(
        by_url(&fx.grabber.list().await.unwrap(), "file")
            .previous
            .is_none()
    );
}

#[tokio::test]
async fn a_link_joins_a_category_of_the_same_name_and_its_folder() {
    let (base, _server) = spawn_server().await;
    let fx = Fixture::new().await;
    let manager = Manager::start(fx.pool.clone(), ManagerConfig::default())
        .await
        .unwrap();
    let default_dest: PathBuf = fx.dir.path().join("downloads");
    let elsewhere = fx.dir.path().join("elsewhere");

    // The first link's category gets its own folder, then is confirmed.
    let first = fx.add(&base, &["first/report.txt"]).await;
    fx.settled().await;
    fx.grabber
        .set_dest_dir(&[first[0].id], Some(&elsewhere))
        .await
        .unwrap();
    let created = fx
        .grabber
        .confirm(&[first[0].id], &default_dest, &manager)
        .await
        .unwrap();

    // A second link with the same category name, added later, takes that
    // folder rather than starting a category in the default folder.
    let second = fx.add(&base, &["second/report.txt"]).await;
    let listed = fx.grabber.list().await.unwrap();
    let joined = listed.iter().find(|l| l.id == second[0].id).unwrap();
    assert_eq!(joined.dest_dir.as_deref(), Some(elsewhere.as_path()));
    fx.settled().await;

    // Confirming it joins the package already in the list, not a new one.
    let again = fx
        .grabber
        .confirm(&[second[0].id], &default_dest, &manager)
        .await
        .unwrap();
    assert_eq!(again, created, "the same package");
    let packages = manager.list().await.unwrap();
    let same_name: Vec<_> = packages.iter().filter(|p| p.name == "report").collect();
    assert_eq!(same_name.len(), 1);
    assert_eq!(same_name[0].downloads.len(), 2);
    manager.shutdown().await;
}
