//! Tool manager tests against a fake GitHub API: install, no-op when current,
//! update with rollback, checksum failures, and unpacking ffmpeg's zip.
//! The binaries are fake, so `verify_runs` is off.

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use rdl_core::db;
use rdl_core::tools::{ToolChannel, ToolName, Tools, ToolsConfig};
use sha2::{Digest, Sha256};
use url::Url;

/// What the fake GitHub (and Codeberg, under `/codeberg/`) serves per repo:
/// release tag and asset bytes.
#[derive(Default)]
struct Releases {
    ytdlp: Mutex<(String, Bytes)>,
    ffmpeg: Mutex<(String, Bytes)>,
    gallerydl: Mutex<(String, Bytes)>,
    /// Serve this checksum instead of the real one.
    wrong_sum: Mutex<bool>,
}

type Shared = (Arc<Releases>, Url);

fn sha(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn release(
    State((r, base)): State<Shared>,
    Path((owner, repo)): Path<(String, String)>,
) -> impl IntoResponse {
    let (tag, asset, sums) = match (&*owner, &*repo) {
        // Only reachable through the Codeberg root.
        ("mikf", "gallery-dl") => (
            r.gallerydl.lock().unwrap().0.clone(),
            "gallery-dl.exe",
            "SHA256SUMS",
        ),
        // No checksum file; GitHub's digest is the only hash.
        ("gdl-org", "builds") => {
            let (tag, data) = r.gallerydl.lock().unwrap().clone();
            let asset = "gallery-dl_windows.exe";
            let json = serde_json::json!({
                "tag_name": tag,
                "assets": [{
                    "name": asset,
                    "browser_download_url": base.join(&format!("dl/{repo}/{asset}")).unwrap().as_str(),
                    "size": data.len(),
                    "digest": format!("sha256:{}", sha(&data)),
                }]
            });
            return (StatusCode::OK, json.to_string());
        }
        ("yt-dlp", "yt-dlp") => (
            r.ytdlp.lock().unwrap().0.clone(),
            "yt-dlp.exe",
            "SHA2-256SUMS",
        ),
        ("yt-dlp", "FFmpeg-Builds") => (
            "latest".to_owned(),
            "ffmpeg-master-latest-win64-gpl-shared.zip",
            "checksums.sha256",
        ),
        _ => return (StatusCode::NOT_FOUND, String::new()),
    };
    let json = serde_json::json!({
        "tag_name": tag,
        "assets": [
            {"name": asset, "browser_download_url": base.join(&format!("dl/{repo}/{asset}")).unwrap().as_str(),
             "size": null, "updated_at": "2026-10-24T08:00:00Z"},
            {"name": sums, "browser_download_url": base.join(&format!("dl/{repo}/{sums}")).unwrap().as_str(),
             "size": 100, "updated_at": "2026-10-24T08:00:00Z"},
        ]
    });
    (StatusCode::OK, json.to_string())
}

async fn asset(
    State((r, _)): State<Shared>,
    Path((repo, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let data = match &*repo {
        "yt-dlp" => r.ytdlp.lock().unwrap().1.clone(),
        "gallery-dl" | "builds" => r.gallerydl.lock().unwrap().1.clone(),
        _ => r.ffmpeg.lock().unwrap().1.clone(),
    };
    if name.contains("SUM") || name.ends_with(".sha256") {
        let hash = if *r.wrong_sum.lock().unwrap() {
            "0".repeat(64)
        } else {
            sha(&data)
        };
        let file = match &*repo {
            "yt-dlp" => "yt-dlp.exe",
            "gallery-dl" => "gallery-dl.exe",
            _ => "ffmpeg-master-latest-win64-gpl-shared.zip",
        };
        return format!("{}  other\n{hash}  {file}\n", "1".repeat(64)).into_response();
    }
    data.into_response()
}

struct Fixture {
    dir: tempfile::TempDir,
    releases: Arc<Releases>,
    tools: Tools,
}

impl Fixture {
    async fn new() -> Self {
        let releases = Arc::new(Releases::default());
        set_ytdlp(&releases, "2026.01.01", b"fake yt-dlp v1");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let app = Router::new()
            .route("/repos/{owner}/{repo}/releases/latest", get(release))
            .route(
                "/codeberg/repos/{owner}/{repo}/releases/latest",
                get(release),
            )
            .route("/dl/{repo}/{name}", get(asset))
            .with_state((Arc::clone(&releases), base.clone()));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(&dir.path().join("t.db")).await.unwrap();
        let config = ToolsConfig {
            codeberg_base: base.join("codeberg/").unwrap(),
            api_base: base,
            verify_runs: false,
            ..ToolsConfig::default()
        };
        let tools = Tools::start(pool, dir.path().join("tools"), config)
            .await
            .unwrap();
        Self {
            dir,
            releases,
            tools,
        }
    }
}

fn set_ytdlp(r: &Releases, tag: &str, data: &[u8]) {
    *r.ytdlp.lock().unwrap() = (tag.to_owned(), Bytes::copy_from_slice(data));
}

#[tokio::test]
async fn installs_then_does_nothing_when_current() {
    let fx = Fixture::new().await;
    assert_eq!(fx.tools.path(ToolName::Ytdlp).await.unwrap(), None);

    let info = fx.tools.install(ToolName::Ytdlp).await.unwrap();
    let path = info.path.clone().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"fake yt-dlp v1");
    assert_eq!(info.version.as_deref(), Some("2026.01.01"));
    assert!(!info.update_available);
    assert!(path.starts_with(fx.dir.path().join("tools").join("yt-dlp")));

    let again = fx.tools.install(ToolName::Ytdlp).await.unwrap();
    assert_eq!(again.path, Some(path));
    assert_eq!(again.installed_at, info.installed_at);
    assert_eq!(again.previous_version, None);
}

#[tokio::test]
async fn updates_keep_the_previous_version_for_rollback() {
    let fx = Fixture::new().await;
    let v1 = fx.tools.install(ToolName::Ytdlp).await.unwrap();

    set_ytdlp(&fx.releases, "2026.02.02", b"fake yt-dlp v2");
    let checked = fx.tools.check(ToolName::Ytdlp).await.unwrap();
    assert!(checked.update_available);
    assert_eq!(checked.latest_release.as_deref(), Some("2026.02.02"));

    let v2 = fx.tools.install(ToolName::Ytdlp).await.unwrap();
    assert_ne!(v2.path, v1.path);
    assert_eq!(
        std::fs::read(v2.path.as_ref().unwrap()).unwrap(),
        b"fake yt-dlp v2"
    );
    assert_eq!(v2.previous_version.as_deref(), Some("2026.01.01"));
    assert!(!v2.update_available);

    let back = fx.tools.rollback(ToolName::Ytdlp).await.unwrap();
    assert_eq!(back.path, v1.path);
    assert_eq!(back.version.as_deref(), Some("2026.01.01"));
    assert_eq!(back.previous_version.as_deref(), Some("2026.02.02"));
    assert!(
        back.update_available,
        "the newer release is still out there"
    );

    // Updating again reuses the kept v2 folder.
    let forward = fx.tools.install(ToolName::Ytdlp).await.unwrap();
    assert_eq!(forward.path, v2.path);
}

#[tokio::test]
async fn a_third_version_prunes_the_oldest() {
    let fx = Fixture::new().await;
    let v1 = fx.tools.install(ToolName::Ytdlp).await.unwrap();
    set_ytdlp(&fx.releases, "2026.02.02", b"v2");
    fx.tools.install(ToolName::Ytdlp).await.unwrap();
    set_ytdlp(&fx.releases, "2026.03.03", b"v3");
    let v3 = fx.tools.install(ToolName::Ytdlp).await.unwrap();

    assert_eq!(v3.previous_version.as_deref(), Some("2026.02.02"));
    assert!(!v1.path.unwrap().exists());
    let folders = std::fs::read_dir(fx.dir.path().join("tools").join("yt-dlp"))
        .unwrap()
        .count();
    assert_eq!(folders, 2);
}

#[tokio::test]
async fn checksum_mismatch_keeps_the_current_install() {
    let fx = Fixture::new().await;
    let v1 = fx.tools.install(ToolName::Ytdlp).await.unwrap();

    set_ytdlp(&fx.releases, "2026.02.02", b"tampered");
    *fx.releases.wrong_sum.lock().unwrap() = true;
    let err = fx.tools.install(ToolName::Ytdlp).await.unwrap_err();
    assert!(err.to_string().contains("checksum mismatch"), "{err}");

    let info = fx.tools.info(ToolName::Ytdlp).await.unwrap();
    assert_eq!(info.path, v1.path);
    assert!(info.error.unwrap().contains("checksum"));
    assert!(info.task.is_none());
    let staging = fx.dir.path().join("tools").join(".staging");
    assert_eq!(std::fs::read_dir(staging).unwrap().count(), 0);
}

#[tokio::test]
async fn unpacks_ffmpeg_bin_folder_from_zip() {
    let fx = Fixture::new().await;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in [
        ("ffmpeg-master/bin/ffmpeg.exe", "ffmpeg"),
        ("ffmpeg-master/bin/ffprobe.exe", "ffprobe"),
        ("ffmpeg-master/bin/avcodec-62.dll", "dll"),
        ("ffmpeg-master/doc/readme.txt", "docs"),
        ("../evil.exe", "nope"),
    ] {
        zip.start_file(name, opts).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    *fx.releases.ffmpeg.lock().unwrap() = (
        "latest".into(),
        Bytes::from(zip.finish().unwrap().into_inner()),
    );

    let info = fx.tools.install(ToolName::Ffmpeg).await.unwrap();
    let exe = info.path.unwrap();
    let bin = exe.parent().unwrap();
    assert_eq!(std::fs::read_to_string(&exe).unwrap(), "ffmpeg");
    assert!(bin.join("ffprobe.exe").is_file());
    assert!(bin.join("avcodec-62.dll").is_file());
    assert!(!bin.join("readme.txt").exists());
    assert_eq!(info.release.as_deref(), Some("2026-10-24"));
}

#[tokio::test]
async fn installs_gallery_dl_from_codeberg_and_nightly_by_digest() {
    let fx = Fixture::new().await;
    *fx.releases.gallerydl.lock().unwrap() = ("v1.32.17".into(), Bytes::from_static(b"gdl"));
    let stable = fx.tools.install(ToolName::Gallerydl).await.unwrap();
    assert_eq!(stable.release.as_deref(), Some("v1.32.17"));
    let exe = stable.path.unwrap();
    assert_eq!(exe.file_name().unwrap(), "gallery-dl.exe");
    assert_eq!(std::fs::read(&exe).unwrap(), b"gdl");

    fx.tools
        .set_channel(ToolName::Gallerydl, ToolChannel::Nightly)
        .await
        .unwrap();
    *fx.releases.gallerydl.lock().unwrap() =
        ("2026.10.25".into(), Bytes::from_static(b"gdl nightly"));
    let nightly = fx.tools.install(ToolName::Gallerydl).await.unwrap();
    assert_eq!(nightly.release.as_deref(), Some("2026.10.25"));
    // Saved under the tool's own name, not the asset's.
    let exe = nightly.path.unwrap();
    assert_eq!(exe.file_name().unwrap(), "gallery-dl.exe");
    assert_eq!(std::fs::read(&exe).unwrap(), b"gdl nightly");
    assert_eq!(nightly.previous_version.as_deref(), Some("v1.32.17"));
}

#[tokio::test]
async fn ensure_installs_once_for_concurrent_callers() {
    let fx = Fixture::new().await;
    let (a, b) = tokio::join!(
        fx.tools.ensure(ToolName::Ytdlp),
        fx.tools.ensure(ToolName::Ytdlp)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(
        fx.tools
            .info(ToolName::Ytdlp)
            .await
            .unwrap()
            .previous_version,
        None
    );
}

#[tokio::test]
async fn forgets_an_install_deleted_from_disk() {
    let fx = Fixture::new().await;
    let info = fx.tools.install(ToolName::Ytdlp).await.unwrap();
    std::fs::remove_file(info.path.unwrap()).unwrap();

    let pool = db::open(&fx.dir.path().join("t.db")).await.unwrap();
    let restarted = Tools::start(pool, fx.dir.path().join("tools"), ToolsConfig::default())
        .await
        .unwrap();
    let info = restarted.info(ToolName::Ytdlp).await.unwrap();
    assert_eq!(info.path, None);
    assert_eq!(info.version, None);
}
