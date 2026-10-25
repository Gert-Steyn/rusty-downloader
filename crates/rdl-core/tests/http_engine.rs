//! End-to-end tests of the HTTP engine against the local test server.

mod common;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{KIB, SIZE, Server, spawn_server};
use rdl_core::Error;
use rdl_core::http::{self, DownloadSpec, Outcome, ProbeResult, Progress, Segment, SegmentConfig};
use reqwest::Url;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------- helpers

struct Run {
    dir: tempfile::TempDir,
    client: reqwest::Client,
    config: SegmentConfig,
}

impl Run {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            client: http::client(None).unwrap(),
            config: SegmentConfig {
                max_connections: 4,
                min_segment_size: 256 * KIB,
                max_retries: 5,
                retry_base_delay: Duration::from_millis(10),
                progress_interval: Duration::from_millis(20),
                throttle: Default::default(),
            },
        }
    }

    fn part(&self) -> PathBuf {
        self.dir.path().join("test.bin.rdl.part")
    }

    fn dest(&self) -> PathBuf {
        self.dir.path().join("out").join("test.bin")
    }

    async fn probe(&self, url: &Url) -> (DownloadSpec, ProbeResult) {
        let spec = DownloadSpec::new(url.clone());
        let probe = http::probe(&self.client, &spec).await.unwrap();
        (spec, probe)
    }

    async fn download(
        &self,
        spec: &DownloadSpec,
        probe: &ProbeResult,
        resume: &[Segment],
        cancel: &CancellationToken,
    ) -> (rdl_core::Result<Outcome>, Progress) {
        let (tx, rx) = watch::channel(Progress::default());
        let result = http::download(
            &self.client,
            spec,
            probe,
            &self.part(),
            &self.dest(),
            resume,
            &self.config,
            &tx,
            cancel,
        )
        .await;
        let last = rx.borrow().clone();
        (result, last)
    }
}

fn assert_downloaded(path: &std::path::Path, server: &Server) {
    common::assert_matches_server(path, server);
}

// ---------------------------------------------------------------- tests

#[tokio::test]
async fn probe_reports_size_ranges_validator_and_filename() {
    let (base, _server) = spawn_server().await;
    let run = Run::new();

    let (_, probe) = run.probe(&base.join("file").unwrap()).await;

    assert_eq!(probe.size, Some(SIZE as u64));
    assert!(probe.accepts_ranges && probe.is_segmentable());
    assert_eq!(probe.validator(), Some("\"v1\""));
    assert_eq!(probe.filename, "test.bin");
}

#[tokio::test]
async fn probe_follows_redirects() {
    let (base, _server) = spawn_server().await;
    let run = Run::new();

    let (_, probe) = run.probe(&base.join("redirect").unwrap()).await;

    assert_eq!(probe.final_url.path(), "/file");
    assert_eq!(probe.filename, "test.bin");
}

#[tokio::test]
async fn probe_maps_gone_to_link_expired() {
    let (base, _server) = spawn_server().await;
    let run = Run::new();
    let spec = DownloadSpec::new(base.join("gone").unwrap());

    let err = http::probe(&run.client, &spec).await.unwrap_err();

    assert!(matches!(err, Error::LinkExpired(410)), "{err:?}");
}

#[tokio::test]
async fn segmented_download_fills_connections_and_matches() {
    let (base, server) = spawn_server().await;
    let run = Run::new();
    let (spec, probe) = run.probe(&base.join("file").unwrap()).await;

    let (result, progress) = run
        .download(&spec, &probe, &[], &CancellationToken::new())
        .await;

    assert_eq!(
        result.unwrap(),
        Outcome::Completed {
            path: run.dest(),
            size: SIZE as u64
        }
    );
    assert_downloaded(&run.dest(), &server);
    assert!(!run.part().exists(), "part file should be renamed away");
    assert!(
        progress.segments.len() >= 4,
        "expected at least one segment per connection, got {}",
        progress.segments.len()
    );
    assert_eq!(progress.downloaded, SIZE as u64);
}

#[tokio::test]
async fn pause_then_resume_completes_without_refetching() {
    let (base, server) = spawn_server().await;
    let run = Run::new();
    let (spec, probe) = run.probe(&base.join("slow").unwrap()).await;

    // Pause once some data has arrived.
    let cancel = CancellationToken::new();
    let (tx, mut rx) = watch::channel(Progress::default());
    let trigger = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                if rx.borrow().downloaded > 512 * KIB {
                    cancel.cancel();
                    break;
                }
            }
        })
    };
    let paused = http::download(
        &run.client,
        &spec,
        &probe,
        &run.part(),
        &run.dest(),
        &[],
        &run.config,
        &tx,
        &cancel,
    )
    .await
    .unwrap();
    trigger.await.unwrap();
    let saved = tx.borrow().clone();

    assert_eq!(paused, Outcome::Paused);
    assert!(run.part().exists() && !run.dest().exists());
    assert!(saved.downloaded > 0 && saved.downloaded < SIZE as u64);

    // Resume from the saved segments against the fast endpoint (same file).
    let served_before = server.bytes_served.load(Ordering::SeqCst);
    let (spec, probe) = run.probe(&base.join("file").unwrap()).await;
    let (result, _) = run
        .download(&spec, &probe, &saved.segments, &CancellationToken::new())
        .await;

    assert!(matches!(result.unwrap(), Outcome::Completed { .. }));
    assert_downloaded(&run.dest(), &server);
    let served_after_resume = server.bytes_served.load(Ordering::SeqCst) - served_before;
    assert!(
        served_after_resume < SIZE as u64,
        "resume should not refetch the whole file ({served_after_resume} bytes served)"
    );
}

#[tokio::test]
async fn resume_detects_changed_remote_file() {
    let (base, server) = spawn_server().await;
    let run = Run::new();
    let (spec, probe) = run.probe(&base.join("file").unwrap()).await;

    // Half-finished state from an earlier session.
    let half = SIZE as u64 / 2;
    std::fs::write(run.part(), vec![0u8; SIZE]).unwrap();
    let saved = [
        Segment {
            start: 0,
            end: half - 1,
            written: half,
        },
        Segment {
            start: half,
            end: SIZE as u64 - 1,
            written: 0,
        },
    ];
    *server.etag.lock().unwrap() = "\"v2\"".into();

    let (result, _) = run
        .download(&spec, &probe, &saved, &CancellationToken::new())
        .await;

    assert!(matches!(result, Err(Error::RemoteChanged)), "{result:?}");
    assert!(!run.dest().exists());
}

#[tokio::test]
async fn invalid_resume_state_restarts_from_scratch() {
    let (base, server) = spawn_server().await;
    let run = Run::new();
    let (spec, probe) = run.probe(&base.join("file").unwrap()).await;
    // Segments that don't match the file size must be ignored, not trusted.
    let bogus = [Segment {
        start: 0,
        end: 99,
        written: 100,
    }];

    let (result, _) = run
        .download(&spec, &probe, &bogus, &CancellationToken::new())
        .await;

    assert!(matches!(result.unwrap(), Outcome::Completed { .. }));
    assert_downloaded(&run.dest(), &server);
}

#[tokio::test]
async fn dropped_connections_are_retried() {
    let (base, server) = spawn_server().await;
    let run = Run::new();
    let (spec, probe) = run.probe(&base.join("flaky").unwrap()).await;

    let (result, _) = run
        .download(&spec, &probe, &[], &CancellationToken::new())
        .await;

    assert!(matches!(result.unwrap(), Outcome::Completed { .. }));
    assert_downloaded(&run.dest(), &server);
    assert_eq!(
        server.flaky_drops_left.load(Ordering::SeqCst),
        0,
        "drops were exercised"
    );
}

#[tokio::test]
async fn server_without_ranges_uses_single_stream() {
    let (base, server) = spawn_server().await;
    let run = Run::new();
    let (spec, probe) = run.probe(&base.join("plain").unwrap()).await;
    assert!(!probe.accepts_ranges);
    assert_eq!(probe.filename, "plain");

    let (result, progress) = run
        .download(&spec, &probe, &[], &CancellationToken::new())
        .await;

    assert!(matches!(result.unwrap(), Outcome::Completed { .. }));
    assert_downloaded(&run.dest(), &server);
    assert_eq!(progress.segments.len(), 1);
}

#[tokio::test]
async fn unknown_size_stream_completes() {
    let (base, server) = spawn_server().await;
    let run = Run::new();
    let (spec, probe) = run.probe(&base.join("chunked").unwrap()).await;
    assert_eq!(probe.size, None);

    let (result, _) = run
        .download(&spec, &probe, &[], &CancellationToken::new())
        .await;

    assert_eq!(
        result.unwrap(),
        Outcome::Completed {
            path: run.dest(),
            size: SIZE as u64
        }
    );
    assert_downloaded(&run.dest(), &server);
}

#[tokio::test]
async fn existing_destination_is_not_overwritten() {
    let (base, _server) = spawn_server().await;
    let run = Run::new();
    let (spec, probe) = run.probe(&base.join("file").unwrap()).await;
    std::fs::create_dir_all(run.dest().parent().unwrap()).unwrap();
    std::fs::write(run.dest(), "keep me").unwrap();

    let (result, _) = run
        .download(&spec, &probe, &[], &CancellationToken::new())
        .await;

    assert!(
        matches!(result, Err(Error::DestinationExists(_))),
        "{result:?}"
    );
    assert_eq!(std::fs::read_to_string(run.dest()).unwrap(), "keep me");
}
