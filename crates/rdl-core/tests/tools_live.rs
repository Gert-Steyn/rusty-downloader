//! Opt-in smoke tests against the real GitHub releases and sites (network,
//! ~110 MB download). Run with
//! `RDL_LIVE_DIR=<folder> cargo test -p rdl-core --test tools_live -- --ignored --nocapture`;
//! the folder keeps the tools between runs.

use std::path::PathBuf;

use rdl_core::db;
use rdl_core::request::RequestContext;
use rdl_core::tools::{ToolName, Tools, ToolsConfig};
use rdl_core::ytdlp::{self, Media, RunEvent, RunOutcome, RunSpec, YtDlpOptions};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Stable, Creative Commons test video. (YouTube asks automated clients to
/// sign in, so it can't be used here.)
const VIDEO: &str = "https://archive.org/details/BigBuckBunny_124";

async fn tools() -> (PathBuf, Tools) {
    let root = PathBuf::from(std::env::var("RDL_LIVE_DIR").expect("set RDL_LIVE_DIR"));
    std::fs::create_dir_all(&root).unwrap();
    let pool = db::open(&root.join("live.db")).await.unwrap();
    let tools = Tools::start(pool, root.join("tools"), ToolsConfig::default())
        .await
        .unwrap();
    (root, tools)
}

#[tokio::test]
#[ignore = "network; downloads yt-dlp and ffmpeg"]
async fn installs_real_tools_and_resolves() {
    let (_root, tools) = tools().await;
    for name in ToolName::ALL {
        let info = tools.install(name).await.unwrap();
        println!("{info:#?}");
        assert!(info.version.is_some());
    }
    let exe = tools.path(ToolName::Ytdlp).await.unwrap().unwrap();
    let media = ytdlp::resolve(
        &exe,
        &Url::parse(VIDEO).unwrap(),
        &RequestContext::default(),
        None,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let Media::Video(info) = media else {
        panic!("expected a video")
    };
    println!("{} formats, title {:?}", info.formats.len(), info.title);
    assert!(!info.formats.is_empty());
}

#[tokio::test]
#[ignore = "network; downloads a short video"]
async fn downloads_real_video() {
    let (root, tools) = tools().await;
    let exe = tools.ensure(ToolName::Ytdlp).await.unwrap();
    let ffmpeg = tools.ensure(ToolName::Ffmpeg).await.unwrap();
    let dest = root.join("out");
    let _ = std::fs::remove_dir_all(&dest);
    let spec = RunSpec {
        exe,
        ffmpeg_dir: ffmpeg.parent().map(Into::into),
        url: Url::parse(VIDEO).unwrap(),
        dest_dir: dest.clone(),
        name: None,
        options: YtDlpOptions {
            preset: ytdlp::FormatPreset::Max480,
            custom_format: None,
        },
        request: RequestContext::default(),
        cookie_file: None,
        limit_rate: None,
        proxy: None,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let printer = tokio::spawn(async move {
        let mut n = 0;
        while let Some(event) = rx.recv().await {
            if matches!(event, RunEvent::Planned { .. }) || n % 20 == 0 {
                println!("{event:?}");
            }
            n += 1;
        }
    });
    let outcome = ytdlp::download(&spec, &tx, &CancellationToken::new())
        .await
        .unwrap();
    drop(tx);
    printer.await.unwrap();
    println!("{outcome:?}");
    assert!(matches!(outcome, RunOutcome::Completed { size, .. } if size > 0));
}

#[tokio::test]
#[ignore = "network; downloads ~47 MB through the manager"]
async fn manager_pauses_and_resumes_a_ytdlp_download() {
    use rdl_core::manager::{
        DEFAULT_QUEUE, DownloadKind, Manager, ManagerConfig, NewDownload, NewPackage, Status,
    };
    use std::time::Duration;

    let (root, tools) = tools().await;
    let dest = root.join("manager-out");
    let _ = std::fs::remove_dir_all(&dest);
    let pool = db::open(&root.join("manager.db")).await.unwrap();
    let manager = Manager::start(
        pool,
        ManagerConfig {
            tools: Some(tools),
            ..ManagerConfig::default()
        },
    )
    .await
    .unwrap();
    let package = manager
        .add_package(NewPackage {
            name: "live".into(),
            dest_dir: dest.clone(),
            queue_id: DEFAULT_QUEUE,
            downloads: vec![NewDownload {
                kind: DownloadKind::Ytdlp,
                ytdlp: Some(YtDlpOptions {
                    preset: ytdlp::FormatPreset::Custom,
                    custom_format: Some("0".into()),
                }),
                title: Some("Big Buck Bunny".into()),
                ..NewDownload::new(Url::parse(VIDEO).unwrap())
            }],
        })
        .await
        .unwrap();
    let id = manager.list().await.unwrap()[0].downloads[0].id;
    let _ = package;

    let wait = |what: &'static str, pred: Box<dyn Fn(&rdl_core::manager::Download) -> bool>| {
        let manager = manager.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(300), async {
                loop {
                    let d = manager.get(id).await.unwrap();
                    if pred(&d) {
                        return d;
                    }
                    if d.status == Status::Failed {
                        panic!("failed while waiting for {what}: {:?}", d.error);
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
        }
    };

    wait("progress", Box::new(|d| d.downloaded > 2_000_000)).await;
    manager.pause(id).await.unwrap();
    let paused = wait("pause", Box::new(|d| d.status == Status::Paused)).await;
    println!(
        "paused at {} bytes, file {:?}",
        paused.downloaded, paused.dest_path
    );
    let partials: Vec<_> = std::fs::read_dir(&dest)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    println!(
        "on disk while paused: {:?}",
        partials.iter().map(|e| e.file_name()).collect::<Vec<_>>()
    );

    manager.resume(id).await.unwrap();
    let done = wait("completion", Box::new(|d| d.status == Status::Completed)).await;
    println!("{done:#?}");
    let path = done.dest_path.unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 46_935_223);
    manager.shutdown().await;
}

/// Installs gallery-dl, lists the schema flags its `--help` lacks, and
/// resolves `RDL_LIVE_GALLERY` (any gallery URL) when set.
#[tokio::test]
#[ignore = "network; downloads gallery-dl"]
async fn gallery_dl_supports_the_flag_schema_and_resolves() {
    use rdl_core::gallerydl;

    let (_root, tools) = tools().await;
    let exe = tools.ensure(ToolName::Gallerydl).await.unwrap();
    println!("{:#?}", tools.info(ToolName::Gallerydl).await.unwrap());
    let flags = gallerydl::supported_flags(&exe).await.unwrap();
    let missing: Vec<&str> = gallerydl::schema()
        .groups
        .iter()
        .flat_map(|g| &g.flags)
        .map(|f| f.flag.as_str())
        .filter(|f| !flags.contains(*f))
        .collect();
    println!("schema flags this gallery-dl lacks: {missing:?}");
    assert!(missing.is_empty(), "the schema is out of date: {missing:?}");
}

/// Resolves `RDL_LIVE_GALLERY` (a public gallery URL), then downloads its
/// first two files twice through the app's runner: the second run must
/// report both as skipped.
#[tokio::test]
#[ignore = "network; set RDL_LIVE_GALLERY"]
async fn gallery_dl_resolves_and_downloads_a_real_gallery() {
    use rdl_core::gallerydl::{self, GalleryDlOptions, RunEvent, RunOutcome, RunSpec};

    let Ok(url) = std::env::var("RDL_LIVE_GALLERY") else {
        println!("RDL_LIVE_GALLERY not set; skipping");
        return;
    };
    let url = Url::parse(&url).unwrap();
    let (root, tools) = tools().await;
    let exe = tools.ensure(ToolName::Gallerydl).await.unwrap();
    let resolved = gallerydl::resolve(
        &exe,
        &url,
        &RequestContext::default(),
        None,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    println!("{:#?}", resolved.gallery);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    assert!(resolved.gallery.files > 0 || resolved.gallery.sub_galleries > 0);

    let dir = root.join("gallery-out");
    let _ = std::fs::remove_dir_all(&dir);
    let archive = root.join("gallery-archive.sqlite3");
    let _ = std::fs::remove_file(&archive);
    let mut options = GalleryDlOptions::default();
    options.flags.insert("range".into(), "1-2".into());
    options.flags.insert("sleep".into(), "1".into());
    let spec = RunSpec {
        exe: exe.clone(),
        ffmpeg_dir: None,
        url,
        dir: dir.clone(),
        archive: Some(archive),
        options,
        available: Some(gallerydl::supported_flags(&exe).await.unwrap()),
        request: RequestContext::default(),
        cookie_file: None,
        limit_rate: None,
        proxy: None,
    };
    println!("args: {:?}", spec.args().unwrap());
    for run in 1..=2 {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let outcome = gallerydl::download(&spec, &tx, &CancellationToken::new()).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        println!("run {run}: {outcome:?}\n{events:#?}");
        let (files, bytes) = outcome_files(&events);
        assert_eq!(outcome.unwrap(), RunOutcome::Completed { files, bytes });
        assert_eq!(events.len(), 2);
        for RunEvent::File { path, skipped, .. } in &events {
            assert!(path.starts_with(&dir), "{}", path.display());
            assert_eq!(*skipped, run == 2);
            assert!(path.is_file());
        }
    }
}

fn outcome_files(events: &[rdl_core::gallerydl::RunEvent]) -> (u32, u64) {
    let bytes = events
        .iter()
        .map(|rdl_core::gallerydl::RunEvent::File { size, .. }| size)
        .sum();
    (events.len() as u32, bytes)
}
