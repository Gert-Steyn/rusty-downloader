//! Local HTTP server shared by the integration tests. Implements byte ranges
//! and `If-Range` properly, plus endpoints that misbehave on purpose.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use std::convert::Infallible;
use std::io;
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
use reqwest::Url;

pub const SIZE: usize = 4 * 1024 * 1024;
pub const KIB: u64 = 1024;
const CHUNK: usize = 16 * 1024;

pub struct Server {
    pub data: Bytes,
    pub etag: Mutex<String>,
    pub bytes_served: AtomicU64,
    pub flaky_drops_left: AtomicUsize,
}

impl Server {
    pub fn served(&self) -> u64 {
        self.bytes_served.load(Ordering::SeqCst)
    }

    pub fn set_etag(&self, etag: &str) {
        *self.etag.lock().unwrap() = etag.to_owned();
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Normal,
    /// 2 ms per 16 KiB chunk, so tests can act mid-transfer.
    Slow,
    /// The first three responses die after 300 KiB.
    Flaky,
}

/// Routes: `/file` `/slow` `/flaky` (ranged, same bytes and ETag), `/plain`
/// (ignores Range), `/chunked` (no length), `/redirect` → `/file`, `/gone` (410).
pub async fn spawn_server() -> (Url, Arc<Server>) {
    let mut data = vec![0u8; SIZE];
    fastrand::Rng::with_seed(42).fill(&mut data);
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

pub fn assert_matches_server(path: &std::path::Path, server: &Server) {
    let got = std::fs::read(path).unwrap();
    assert_eq!(got.len(), server.data.len(), "size differs");
    assert!(got == server.data, "content differs");
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
    let chunks: Vec<Result<Bytes, Infallible>> = server
        .data
        .chunks(CHUNK)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    Body::from_stream(stream::iter(chunks)).into_response()
}
