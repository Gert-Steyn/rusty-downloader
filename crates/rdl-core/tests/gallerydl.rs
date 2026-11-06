//! gallery-dl downloads against a fake gallery-dl (a batch file) that
//! behaves like the real one with stdout piped: it prints each finished
//! file's path, `# ` before files that already exist, and logs errors as
//! `[extractor][error] ...` on stderr. The URL picks the behaviour:
//! `/slow` waits after the first file, `/fail` fails after it. A cookie
//! file it's given is copied to `cookies.txt` in the gallery folder; when
//! resolving, it calls the album "Fake album signed in" instead.
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use rdl_core::db;
use rdl_core::gallerydl::{self, GalleryDlOptions, RunEvent, RunOutcome, RunSpec};
use rdl_core::manager::{
    DEFAULT_QUEUE, DownloadKind, FileCount, Manager, ManagerConfig, NewDownload, NewPackage, Status,
};
use rdl_core::request::{Cookie, RequestContext};
use rdl_core::tools::{Tools, ToolsConfig};
use tokio_util::sync::CancellationToken;
use url::Url;

const FAKE: &str = r#"@echo off
setlocal
if "%~1"=="--help" exit /b 0
:args
if "%~1"=="" goto run
if "%~1"=="-j" goto json
if "%~1"=="-D" set "DIR=%~2"
if "%~1"=="--cookies" set "COOKIES=%~2"
set "URL=%~1"
shift
goto args
:run
if defined COOKIES copy /y "%COOKIES%" "%DIR%\cookies.txt" >NUL
set FRESH=1
if exist "%DIR%\001.jpg" set FRESH=0
if exist "%DIR%\001.jpg" (echo # %DIR%\001.jpg) else (
  echo one>"%DIR%\001.jpg"
  echo %DIR%\001.jpg
)
if "%URL%"=="https://gallery.test/fail" (
  echo [gallery][error] HttpError: '503 Service Unavailable' 1>&2
  exit /b 4
)
if "%URL%"=="https://gallery.test/slow" if "%FRESH%"=="1" ping -n 30 127.0.0.1 >NUL
if exist "%DIR%\002.jpg" (echo # %DIR%\002.jpg) else (
  echo two two>"%DIR%\002.jpg"
  echo %DIR%\002.jpg
)
exit /b 0
:json
set "TITLE=Fake album"
if defined COOKIES if exist "%COOKIES%" set "TITLE=Fake album signed in"
echo [[2, {"category": "fake", "subcategory": "album", "title": "%TITLE%"}], [3, "https://gallery.test/1.jpg", {"extension": "jpg", "filesize": 5}], [3, "https://gallery.test/2.jpg", {"extension": "jpg", "filesize": 9}]]
exit /b 0
"#;

/// A yt-dlp that knows no site at all.
const FAKE_YTDLP: &str = "@echo off\r\necho ERROR: Unsupported URL: %~1 1>&2\r\nexit /b 1\r\n";

fn fake_tool(dir: &Path) -> PathBuf {
    let exe = dir.join("gallery-dl.cmd");
    std::fs::write(&exe, FAKE.replace('\n', "\r\n")).unwrap();
    exe
}

fn spec(exe: &Path, dir: &Path, url: &str) -> RunSpec {
    RunSpec {
        exe: exe.to_owned(),
        ffmpeg_dir: None,
        url: Url::parse(url).unwrap(),
        dir: dir.to_owned(),
        archive: None,
        options: GalleryDlOptions::default(),
        available: None,
        request: RequestContext::default(),
        cookie_file: None,
        limit_rate: None,
        connection: Default::default(),
    }
}

async fn run(spec: &RunSpec) -> (rdl_core::Result<RunOutcome>, Vec<RunEvent>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let outcome = gallerydl::download(spec, &tx, &CancellationToken::new()).await;
    drop(tx);
    let mut events = Vec::new();
    while let Some(e) = rx.recv().await {
        events.push(e);
    }
    (outcome, events)
}

#[tokio::test]
async fn reports_files_and_skips_existing_ones_on_a_second_run() {
    let root = tempfile::tempdir().unwrap();
    let exe = fake_tool(root.path());
    let dir = root.path().join("Trip");
    let spec = spec(&exe, &dir, "https://gallery.test/album");

    let (outcome, events) = run(&spec).await;
    assert_eq!(
        outcome.unwrap(),
        RunOutcome::Completed {
            files: 2,
            bytes: 5 + 9
        }
    );
    assert_eq!(
        events,
        [
            RunEvent::File {
                path: dir.join("001.jpg"),
                size: 5,
                skipped: false
            },
            RunEvent::File {
                path: dir.join("002.jpg"),
                size: 9,
                skipped: false
            },
        ]
    );

    let (outcome, events) = run(&spec).await;
    assert!(matches!(
        outcome.unwrap(),
        RunOutcome::Completed { files: 2, .. }
    ));
    assert!(
        events
            .iter()
            .all(|e| matches!(e, RunEvent::File { skipped: true, .. }))
    );
}

#[tokio::test]
async fn failures_carry_the_logged_error() {
    let root = tempfile::tempdir().unwrap();
    let exe = fake_tool(root.path());
    let (outcome, events) = run(&spec(
        &exe,
        &root.path().join("g"),
        "https://gallery.test/fail",
    ))
    .await;
    let err = outcome.unwrap_err().to_string();
    assert!(err.contains("503 Service Unavailable"), "{err}");
    assert!(err.contains("after 1 files"), "{err}");
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn cancel_kills_the_run() {
    let root = tempfile::tempdir().unwrap();
    let exe = fake_tool(root.path());
    let spec = spec(&exe, &root.path().join("g"), "https://gallery.test/slow");
    let cancel = CancellationToken::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let task = {
        let cancel = cancel.clone();
        tokio::spawn(async move { gallerydl::download(&spec, &tx, &cancel).await })
    };
    rx.recv().await.expect("first file");
    let started = std::time::Instant::now();
    cancel.cancel();
    assert_eq!(task.await.unwrap().unwrap(), RunOutcome::Paused);
    assert!(started.elapsed() < Duration::from_secs(5));
}

struct Fixture {
    root: tempfile::TempDir,
    manager: Manager,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let exe = fake_tool(root.path());
        let pool = db::open(&root.path().join("t.db")).await.unwrap();
        let tools = Tools::start(
            pool.clone(),
            root.path().join("tools"),
            ToolsConfig::default(),
        )
        .await
        .unwrap();
        // Pretend gallery-dl is installed: point its row at the fake.
        sqlx::query("UPDATE tools SET path = ?, version = 'fake' WHERE name = 'gallery-dl'")
            .bind(exe.to_string_lossy().into_owned())
            .execute(&pool)
            .await
            .unwrap();
        let manager = Manager::start(
            pool,
            ManagerConfig {
                tools: Some(tools),
                archives_dir: Some(root.path().join("archives")),
                save_interval: Duration::from_millis(30),
                rescan_interval: Duration::from_millis(100),
                ..ManagerConfig::default()
            },
        )
        .await
        .unwrap();
        Self { root, manager }
    }

    async fn add(&self, url: &str, files_total: Option<u32>) -> i64 {
        self.manager
            .add_package(NewPackage {
                name: "Trip".into(),
                dest_dir: self.root.path().join("out"),
                queue_id: DEFAULT_QUEUE,
                downloads: vec![NewDownload {
                    kind: DownloadKind::Gallerydl,
                    filename: Some("Mountain trip".into()),
                    gallerydl: Some(GalleryDlOptions::default()),
                    files_total,
                    ..NewDownload::new(Url::parse(url).unwrap())
                }],
            })
            .await
            .unwrap();
        self.manager.list().await.unwrap()[0].downloads[0].id
    }

    async fn wait(&self, id: i64, status: Status) -> rdl_core::manager::Download {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let d = self.manager.get(id).await.unwrap();
                if d.status == status {
                    return d;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {status:?}"))
    }
}

#[tokio::test]
async fn manager_downloads_a_gallery_into_its_folder_and_deletes_only_its_files() {
    let fx = Fixture::new().await;
    let folder = fx.root.path().join("out").join("Mountain trip");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("mine.txt"), "not from the gallery").unwrap();

    let id = fx.add("https://gallery.test/album", Some(2)).await;
    let done = fx.wait(id, Status::Completed).await;
    assert_eq!(done.dest_path.as_deref(), Some(folder.as_path()));
    assert_eq!(
        done.files,
        Some(FileCount {
            done: 2,
            total: Some(2)
        })
    );
    assert_eq!((done.downloaded, done.size), (14, Some(14)));
    assert!(folder.join("001.jpg").is_file());

    // Found in the app: a second download of the gallery into the same
    // folder skips every file, yet "delete files" must still remove them.
    fx.manager.remove(id, false).await.unwrap();
    let id = fx.add("https://gallery.test/album", Some(2)).await;
    let again = fx.wait(id, Status::Completed).await;
    assert_eq!(again.files.map(|f| f.done), Some(2));
    fx.manager.remove(id, true).await.unwrap();
    assert!(!folder.join("001.jpg").exists());
    assert!(!folder.join("002.jpg").exists());
    assert!(
        folder.join("mine.txt").is_file(),
        "files the gallery did not write stay"
    );
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn manager_pauses_and_resumes_a_gallery() {
    let fx = Fixture::new().await;
    let id = fx.add("https://gallery.test/slow", None).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        while fx
            .manager
            .get(id)
            .await
            .unwrap()
            .files
            .is_none_or(|f| f.done == 0)
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("first file");
    fx.manager.pause(id).await.unwrap();
    let paused = fx.wait(id, Status::Paused).await;
    assert_eq!(
        paused.files,
        Some(FileCount {
            done: 1,
            total: None
        })
    );

    fx.manager.resume(id).await.unwrap();
    let done = fx.wait(id, Status::Completed).await;
    // The first file was skipped this time but still counts, and once done
    // the total is known.
    assert_eq!(
        done.files,
        Some(FileCount {
            done: 2,
            total: Some(2)
        })
    );
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn rerunning_a_completed_gallery_fetches_into_the_same_folder() {
    let fx = Fixture::new().await;
    let folder = fx.root.path().join("out").join("Mountain trip");
    let id = fx.add("https://gallery.test/album", Some(2)).await;
    let first = fx.wait(id, Status::Completed).await;
    assert_eq!(first.dest_path.as_deref(), Some(folder.as_path()));

    fx.manager.rerun_gallery(id).await.unwrap();
    let again = fx.wait(id, Status::Completed).await;
    // The same folder, and gallery-dl reports both files as already there.
    assert_eq!(again.dest_path.as_deref(), Some(folder.as_path()));
    assert_eq!(
        again.files,
        Some(FileCount {
            done: 2,
            total: Some(2)
        })
    );
    assert!(folder.join("001.jpg").is_file());
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn rerun_leaves_a_download_that_is_not_completed_alone() {
    let fx = Fixture::new().await;
    let id = fx.add("https://gallery.test/slow", None).await;
    fx.manager.pause(id).await.unwrap();
    fx.wait(id, Status::Paused).await;

    fx.manager.rerun_gallery(id).await.unwrap();
    assert_eq!(fx.manager.get(id).await.unwrap().status, Status::Paused);
    fx.manager.shutdown().await;
}

#[tokio::test]
async fn manager_records_gallery_failures_and_can_delete_what_finished() {
    let fx = Fixture::new().await;
    let id = fx.add("https://gallery.test/fail", None).await;
    let failed = fx.wait(id, Status::Failed).await;
    assert!(failed.error.unwrap().contains("503"));
    let first = failed.dest_path.unwrap().join("001.jpg");
    assert!(first.is_file());

    // Found in the app: a gallery that failed part-way kept its finished files.
    fx.manager.remove(id, true).await.unwrap();
    assert!(!first.exists());
    assert!(
        !first.parent().unwrap().exists(),
        "the emptied folder goes too"
    );
    fx.manager.shutdown().await;
}

/// Fake yt-dlp and gallery-dl installed in `root`, and tools that use them.
async fn fake_tools(root: &Path) -> (db::Pool, Tools) {
    let gallery_dl = fake_tool(root);
    let ytdlp = root.join("yt-dlp.cmd");
    std::fs::write(&ytdlp, FAKE_YTDLP).unwrap();
    let pool = db::open(&root.join("t.db")).await.unwrap();
    let tools = Tools::start(pool.clone(), root.join("tools"), ToolsConfig::default())
        .await
        .unwrap();
    for (name, exe) in [("yt-dlp", &ytdlp), ("gallery-dl", &gallery_dl)] {
        sqlx::query("UPDATE tools SET path = ?, version = 'fake' WHERE name = ?")
            .bind(exe.to_string_lossy().into_owned())
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    (pool, tools)
}

/// The first link in the grabber, once its check is done.
async fn checked_link(
    grabber: &rdl_core::linkgrabber::LinkGrabber,
) -> rdl_core::linkgrabber::GrabbedLink {
    use rdl_core::linkgrabber::Availability;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let links = grabber.list().await.unwrap();
            if !matches!(
                links[0].availability,
                Availability::Unknown | Availability::Checking
            ) {
                return links[0].clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("link check finished")
}

#[tokio::test]
async fn browser_cookies_go_to_gallery_dl_when_checking_and_downloading() {
    use rdl_core::linkgrabber::{LinkGrabber, LinkSource};

    let root = tempfile::tempdir().unwrap();
    let (pool, tools) = fake_tools(root.path()).await;
    let grabber = LinkGrabber::start(pool.clone(), None, Some(tools.clone()))
        .await
        .unwrap();
    let request = RequestContext {
        cookies: vec![Cookie {
            name: "sid".into(),
            value: "ok".into(),
            domain: "127.0.0.1".into(),
            host_only: true,
            path: "/".into(),
            secure: false,
            http_only: false,
            expires: None,
        }],
        ..Default::default()
    };
    let url = Url::parse("http://127.0.0.1:9/album/42").unwrap();
    grabber
        .add_with_request(vec![url], LinkSource::Manual, request.clone())
        .await
        .unwrap();
    let link = checked_link(&grabber).await;
    assert_eq!(link.filename.as_deref(), Some("Fake album signed in"));

    let manager = Manager::start(
        pool,
        ManagerConfig {
            tools: Some(tools),
            rescan_interval: Duration::from_millis(100),
            ..ManagerConfig::default()
        },
    )
    .await
    .unwrap();
    let out = root.path().join("out");
    grabber.confirm(&[link.id], &out, &manager).await.unwrap();
    let id = manager.list().await.unwrap()[0].downloads[0].id;
    tokio::time::timeout(Duration::from_secs(20), async {
        while manager.get(id).await.unwrap().status != Status::Completed {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("gallery finished");
    let seen =
        std::fs::read_to_string(out.join("Fake album signed in").join("cookies.txt")).unwrap();
    assert_eq!(seen, request.netscape());
    manager.shutdown().await;
}

#[tokio::test]
async fn linkgrabber_offers_pages_yt_dlp_rejects_to_gallery_dl() {
    use rdl_core::linkgrabber::{Availability, LinkGrabber, LinkSource};

    let root = tempfile::tempdir().unwrap();
    let (pool, tools) = fake_tools(root.path()).await;
    let grabber = LinkGrabber::start(pool.clone(), None, Some(tools.clone()))
        .await
        .unwrap();

    // Nothing listens on port 9: the probe fails, which is worth asking the tools about.
    let url = Url::parse("http://127.0.0.1:9/album/42").unwrap();
    grabber.add(vec![url], LinkSource::Manual).await.unwrap();
    let link = checked_link(&grabber).await;

    assert_eq!(link.availability, Availability::Online, "{:?}", link.error);
    assert_eq!(link.kind, DownloadKind::Gallerydl);
    assert_eq!(link.filename.as_deref(), Some("Fake album"));
    assert_eq!(link.package, "Fake album");
    assert_eq!(link.size, Some(14));
    let gallery = link.gallery.clone().unwrap();
    assert_eq!(
        (gallery.files, gallery.category.as_deref()),
        (2, Some("fake"))
    );
    assert_eq!(link.gallerydl, Some(GalleryDlOptions::default()));

    let mut options = GalleryDlOptions::default();
    options.flags.insert("range".into(), "1-5".into());
    grabber
        .set_gallery_options(&[link.id], &options)
        .await
        .unwrap();
    options.flags.insert("retries".into(), "many".into());
    assert!(
        grabber
            .set_gallery_options(&[link.id], &options)
            .await
            .is_err()
    );
    // Drive-relative: would land under the app's working folder.
    assert!(
        grabber
            .set_dest_dir(&[link.id], Some(Path::new("C:Users\\x")))
            .await
            .is_err()
    );

    let manager = Manager::start(
        pool,
        ManagerConfig {
            tools: Some(tools),
            ..ManagerConfig::default()
        },
    )
    .await
    .unwrap();
    manager.pause_all().await.unwrap();
    grabber
        .confirm(&[link.id], &root.path().join("out"), &manager)
        .await
        .unwrap();
    let download = manager.list().await.unwrap()[0].downloads[0].clone();
    assert_eq!(download.kind, "gallerydl");
    // Found in the app: with a range set, the resolved 82 files read "5 of 82".
    assert_eq!(download.files.map(|f| f.total), Some(None));
    assert_eq!(download.size, None);
    assert_eq!(download.gallerydl.unwrap().flags["range"], "1-5");

    manager.shutdown().await;
}

/// A yt-dlp whose catch-all extractor finds one embedded video on any page.
const FAKE_YTDLP_GENERIC: &str = "@echo off\r\necho {\"_type\": \"video\", \"id\": \"x\", \"title\": \"Embedded clip\", \"extractor_key\": \"HTML5MediaEmbed\", \"formats\": []}\r\nexit /b 0\r\n";

/// A gallery-dl without an extractor for anything.
const FAKE_GDL_UNSUPPORTED: &str =
    "@echo off\r\necho [gallery-dl][error] Unsupported URL 1>&2\r\nexit /b 64\r\n";

/// Check one link with the given fake tools and return it once settled.
async fn check_link(
    ytdlp_script: &str,
    gallery_dl_script: Option<&str>,
) -> rdl_core::linkgrabber::GrabbedLink {
    use rdl_core::linkgrabber::{Availability, LinkGrabber, LinkSource};

    let root = tempfile::tempdir().unwrap();
    let gallery_dl = match gallery_dl_script {
        Some(script) => {
            let exe = root.path().join("gallery-dl.cmd");
            std::fs::write(&exe, script).unwrap();
            exe
        }
        None => fake_tool(root.path()),
    };
    let ytdlp = root.path().join("yt-dlp.cmd");
    std::fs::write(&ytdlp, ytdlp_script).unwrap();
    let pool = db::open(&root.path().join("t.db")).await.unwrap();
    let tools = Tools::start(
        pool.clone(),
        root.path().join("tools"),
        ToolsConfig::default(),
    )
    .await
    .unwrap();
    for (name, exe) in [("yt-dlp", &ytdlp), ("gallery-dl", &gallery_dl)] {
        sqlx::query("UPDATE tools SET path = ?, version = 'fake' WHERE name = ?")
            .bind(exe.to_string_lossy().into_owned())
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    let grabber = LinkGrabber::start(pool, None, Some(tools)).await.unwrap();
    let url = Url::parse("http://127.0.0.1:9/wiki/Category:Clips").unwrap();
    grabber.add(vec![url], LinkSource::Manual).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let links = grabber.list().await.unwrap();
            if !matches!(
                links[0].availability,
                Availability::Unknown | Availability::Checking
            ) {
                return links[0].clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("link check finished")
}

/// Found with gallery-dl 1.32.17 in the app: yt-dlp's HTML5 embed extractor
/// offered the one video on a Wikimedia category of 82 files.
#[tokio::test]
async fn a_gallery_beats_a_video_from_yt_dlps_catch_all_extractor() {
    let link = check_link(FAKE_YTDLP_GENERIC, None).await;
    assert_eq!(link.kind, DownloadKind::Gallerydl, "{link:?}");
    assert_eq!(link.filename.as_deref(), Some("Fake album"));
}

#[tokio::test]
async fn the_catch_all_video_is_kept_when_gallery_dl_has_nothing() {
    let link = check_link(FAKE_YTDLP_GENERIC, Some(FAKE_GDL_UNSUPPORTED)).await;
    assert_eq!(link.kind, DownloadKind::Ytdlp, "{link:?}");
    assert_eq!(link.filename.as_deref(), Some("Embedded clip"));
}
