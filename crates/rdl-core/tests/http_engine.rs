//! End-to-end tests of the HTTP engine against a local server that implements
//! byte ranges, `If-Range` and a few deliberately misbehaving endpoints.

use std::convert::Infallible;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use futures_util::stream::{self, StreamExt};
use rdl_core::Error;
use rdl_core::http::{self, DownloadSpec, Outcome, ProbeResult, Progress, Segment, SegmentConfig};
use reqwest::Url;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const SIZE: usize = 4 * 1024 * 1024;
const CHUNK: usize = 16 * 1024;
const KIB: u64 = 1024;

// ---------------------------------------------------------------- test server

struct Server {
    data: Bytes,
    etag: Mutex<String>,
    bytes_served: AtomicU64,
    flaky_drops_left: AtomicUsize,
}

#[derive(Clone, Copy)]
enum Mode {
    Normal,
    Slow,
    Flaky,
}

async fn spawn_server() -> (Url, Arc<Server>) {
    let mut data = vec![0u8; SIZE];
    let mut rng = fastrand::Rng::with_seed(42);
    rng.fill(&mut data);
    let server = Arc::new(Server {
        data: Bytes::from(data),
        etag: Mutex::new("\"v1\"".into()),
        bytes_served: AtomicU64::new(0),
        flaky_drops_left: AtomicUsize::new(3),
    });

    let app = Router::new()
        .route("/file", get(|s, h| ranged(s, h, Mode::Normal)))
        .route("/slow", get(|s, h| ranged(s, h, Mode::Slow)))
        .route("/flaky", get(|s, h| ranged(s, h, Mode::Flaky)))
        .route("/plain", get(plain))
        .route("/chunked", get(chunked))
        .route("/redirect", get(|| async { Redirect::temporary("/file") }))
        .route("/gone", get(|| async { StatusCode::GONE }))
        .with_state(Arc::clone(&server));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (Url::parse(&format!("http://{addr}/")).unwrap(), server)
}

fn parse_range(headers: &HeaderMap, len: u64) -> Option<(u64, u64)> {
    let spec = headers
        .get(header::RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: u64 = start.parse().ok()?;
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<u64>().ok()?.min(len - 1)
    };
    (start <= end).then_some((start, end))
}

/// Serve `data[start..=end]` in chunks, optionally slowly or dying part-way.
fn body(server: &Arc<Server>, start: u64, end: u64, mode: Mode) -> Body {
    let slice = server.data.slice(start as usize..=end as usize);
    let drop_after = match mode {
        Mode::Flaky if server.flaky_drops_left.load(Ordering::SeqCst) > 0 => {
            server.flaky_drops_left.fetch_sub(1, Ordering::SeqCst);
            Some(300 * KIB as usize)
        }
        _ => None,
    };
    let server = Arc::clone(server);
    let chunks = (0..slice.len())
        .step_by(CHUNK)
        .map(move |off| (off, slice.slice(off..(off + CHUNK).min(slice.len()))));
    let stream = stream::iter(chunks).then(move |(off, chunk)| {
        let server = Arc::clone(&server);
        async move {
            if matches!(mode, Mode::Slow) {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            if drop_after.is_some_and(|limit| off >= limit) {
                return Err(io::Error::other("simulated connection drop"));
            }
            server
                .bytes_served
                .fetch_add(chunk.len() as u64, Ordering::SeqCst);
            Ok::<_, io::Error>(chunk)
        }
    });
    Body::from_stream(stream)
}

async fn ranged(State(server): State<Arc<Server>>, headers: HeaderMap, mode: Mode) -> Response {
    let len = server.data.len() as u64;
    let etag = server.etag.lock().unwrap().clone();
    let if_range_ok = headers
        .get(header::IF_RANGE)
        .is_none_or(|v| v.to_str().ok() == Some(etag.as_str()));

    let common = [
        (header::ETAG, etag),
        (header::ACCEPT_RANGES, "bytes".into()),
        (
            header::CONTENT_DISPOSITION,
            r#"attachment; filename="test.bin""#.into(),
        ),
    ];
    match parse_range(&headers, len).filter(|_| if_range_ok) {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            common,
            [
                (header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}")),
                (header::CONTENT_LENGTH, (end - start + 1).to_string()),
            ],
            body(&server, start, end, mode),
        )
            .into_response(),
        None => (
            StatusCode::OK,
            common,
            [(header::CONTENT_LENGTH, len.to_string())],
            body(&server, 0, len - 1, mode),
        )
            .into_response(),
    }
}

/// Ignores Range entirely.
async fn plain(State(server): State<Arc<Server>>) -> Response {
    let len = server.data.len() as u64;
    (
        [(header::CONTENT_LENGTH, len.to_string())],
        body(&server, 0, len - 1, Mode::Normal),
    )
        .into_response()
}

/// No ranges and no Content-Length (chunked transfer encoding).
async fn chunked(State(server): State<Arc<Server>>) -> Response {
    let data = server.data.clone();
    let chunks: Vec<Result<Bytes, Infallible>> = data
        .chunks(CHUNK)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    Body::from_stream(stream::iter(chunks)).into_response()
}

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
    let got = std::fs::read(path).unwrap();
    assert_eq!(got.len(), server.data.len(), "size differs");
    assert!(got == server.data, "content differs");
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
