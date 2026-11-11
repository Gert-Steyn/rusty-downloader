//! Running a `kind = 'torrent'` download: get the torrent into the engine,
//! record its name and size once they're known, and mirror its progress
//! until it's complete.
//!
//! The files go where a torrent normally would, but under the package's
//! folder: a single-file torrent is `<package folder>/<name>`, and a
//! multi-file one is the folder `<package folder>/<name>` holding its files.
//! Pause pauses the torrent in the engine; resuming finds it again by its
//! info hash (or re-adds it, which re-checks the files already there).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::HeaderMap;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::store::{self, DownloadRow, Resolved};
use super::{Event, Inner};
use crate::http::{self, DownloadSpec, Outcome, filename};
use crate::torrent::{Inspection, ManagedTorrent, TorrentEngine, TorrentSource, TorrentStatsState};
use crate::{Error, Result};

/// How often progress is read from the engine.
const POLL: Duration = Duration::from_millis(500);

/// Largest `.torrent` file fetched over HTTP.
const MAX_TORRENT_FILE: usize = 16 * 1024 * 1024;

pub(super) async fn execute(
    inner: &Arc<Inner>,
    row: &DownloadRow,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let engine = inner
        .torrents
        .as_ref()
        .ok_or_else(|| Error::Unsupported("torrent downloads without a torrent folder".into()))?;
    let pool = &inner.pool;
    let url = Url::parse(&row.source_url)
        .map_err(|e| Error::InvalidUrl(format!("{}: {e}", row.source_url)))?;

    // A torrent the engine already has (from an earlier run, or this one)
    // carries on from where it was.
    let known = match &row.info_hash {
        Some(info_hash) => engine.find_for_resume(info_hash).await?,
        None => None,
    };
    let (handle, dest) = match known {
        Some(handle) => {
            let dest = match &row.dest_path {
                Some(path) => PathBuf::from(path),
                None => {
                    let package = package_dir(inner, row).await?;
                    package.join(handle.name().unwrap_or_default())
                }
            };
            (handle, dest)
        }
        None => {
            let package = package_dir(inner, row).await?;
            let source = tokio::select! {
                _ = cancel.cancelled() => return Ok(Outcome::Paused),
                source = read_source(inner, row, &url) => source?,
            };
            let info = tokio::select! {
                _ = cancel.cancelled() => return Ok(Outcome::Paused),
                info = engine.inspect(source) => info?,
            };
            let name = entry_name(&info);
            let (folder, dest) = if info.files > 1 {
                let folder = package.join(&name);
                (folder.clone(), folder)
            } else {
                (package.clone(), package.join(&name))
            };
            {
                let _claim = inner.dest_lock.lock().await;
                // A resumed download already owns its path; a new one may not
                // take a path another download is using.
                if row.dest_path.is_none() && store::dest_taken(pool, &dest).await? {
                    return Err(Error::InvalidInput(format!(
                        "another download already uses {}",
                        dest.display()
                    )));
                }
                store::set_info_hash(pool, row.id, &info.info_hash).await?;
                store::set_resolved(
                    pool,
                    row.id,
                    &Resolved {
                        resolved_url: &row.source_url,
                        filename: &name,
                        dest_path: &dest,
                        size: Some(info.size),
                        etag: None,
                        last_modified: None,
                    },
                )
                .await?;
            }
            inner.emit(Event::Resolved {
                download_id: row.id,
                filename: name,
                size: Some(info.size),
            });
            let handle = tokio::select! {
                _ = cancel.cancelled() => return Ok(Outcome::Paused),
                handle = engine.add(TorrentSource::Bytes(info.torrent), &folder, false) => handle?,
            };
            (handle, dest)
        }
    };

    engine.resume(&handle).await?;
    let Some(size) = run(inner, engine, row, &handle, cancel).await? else {
        return Ok(Outcome::Paused);
    };
    // Seeding comes later; a finished torrent is paused until then.
    if let Err(e) = engine.pause(&handle).await {
        tracing::warn!(download = row.id, error = %e, "could not pause finished torrent");
    }
    Ok(Outcome::Completed { path: dest, size })
}

/// Mirror the torrent's progress until it's complete: `Some(size)` then.
/// `None` means it was cancelled: the torrent is paused and the progress
/// saved. An engine failure is an error.
async fn run(
    inner: &Arc<Inner>,
    engine: &TorrentEngine,
    row: &DownloadRow,
    handle: &Arc<ManagedTorrent>,
    cancel: &CancellationToken,
) -> Result<Option<u64>> {
    let pool = &inner.pool;
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_save = Instant::now();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                if let Err(e) = engine.pause(handle).await {
                    tracing::warn!(download = row.id, error = %e, "could not pause torrent");
                }
                store::set_downloaded(pool, row.id, handle.stats().progress_bytes).await?;
                return Ok(None);
            }
            _ = tick.tick() => {}
        }
        let stats = handle.stats();
        if matches!(stats.state, TorrentStatsState::Error) {
            let message = stats
                .error
                .clone()
                .unwrap_or_else(|| "the torrent stopped with an error".into());
            // Dropped from the engine, so a retry adds it again from scratch.
            if let Err(e) = engine.forget(&handle.info_hash().as_string(), false).await {
                tracing::warn!(download = row.id, error = %e, "could not drop failed torrent");
            }
            return Err(Error::Torrent(message));
        }
        if stats.finished {
            emit(inner, row.id, &stats);
            return Ok(Some(stats.total_bytes));
        }
        if matches!(stats.state, TorrentStatsState::Paused) {
            // Only the engine itself could have paused it: carry on.
            engine.resume(handle).await?;
        }
        emit(inner, row.id, &stats);
        if last_save.elapsed() >= inner.config.save_interval {
            last_save = Instant::now();
            if let Err(e) = store::set_downloaded(pool, row.id, stats.progress_bytes).await {
                tracing::warn!(download = row.id, error = %e, "could not save progress");
            }
        }
    }
}

fn emit(inner: &Inner, id: i64, stats: &crate::torrent::TorrentStats) {
    let live = stats.live.as_ref();
    inner.emit(Event::Progress {
        download_id: id,
        downloaded: stats.progress_bytes,
        total: Some(stats.total_bytes),
        speed_bps: live.map_or(0, |l| l.download_speed.as_bytes()),
        connections: live.map_or(0, |l| l.snapshot.peer_stats.live as usize),
        files: None,
    });
}

/// The package's folder, which torrents download into.
async fn package_dir(inner: &Inner, row: &DownloadRow) -> Result<PathBuf> {
    let package = store::get_package(&inner.pool, row.package_id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("package {}", row.package_id)))?;
    Ok(PathBuf::from(package.dest_dir))
}

/// The torrent's `.torrent` contents: from the magnet itself, a `.torrent`
/// link, or a `file://` path.
async fn read_source(inner: &Inner, row: &DownloadRow, url: &Url) -> Result<TorrentSource> {
    match url.scheme() {
        "magnet" => Ok(TorrentSource::Magnet(row.source_url.clone())),
        "file" => {
            let path = url
                .to_file_path()
                .map_err(|()| Error::InvalidUrl(row.source_url.clone()))?;
            Ok(TorrentSource::Bytes(tokio::fs::read(&path).await?))
        }
        _ => {
            let client = inner.network.client(url, row.proxy())?;
            let (request, login) = inner.network.with_account(url, row.request());
            let spec = DownloadSpec::with_request(url.clone(), request).with_login(login);
            let response = http::get(&client, &spec, url, &HeaderMap::new()).await?;
            http::check_status(response.status(), url)?;
            let bytes = response.bytes().await?;
            if bytes.len() > MAX_TORRENT_FILE {
                return Err(Error::Torrent(
                    "the .torrent file is too large to be one".into(),
                ));
            }
            Ok(TorrentSource::Bytes(bytes.to_vec()))
        }
    }
}

/// The name the files go under: the torrent's own, or its info hash if the
/// name can't be a single folder or file name. A multi-file torrent's folder
/// is ours to name, so characters Windows refuses (`:`, `?`, `*` …) are
/// replaced; a single file keeps the name the engine writes it under.
fn entry_name(info: &Inspection) -> String {
    let name = info.name.trim();
    let unsafe_name = name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']);
    if unsafe_name {
        info.info_hash.clone()
    } else if info.files > 1 {
        let clean = filename::sanitize(name);
        if clean.is_empty() {
            info.info_hash.clone()
        } else {
            clean
        }
    } else {
        name.to_owned()
    }
}
