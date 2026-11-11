//! Seeding: a finished torrent stays in the engine and keeps uploading until
//! its limits are met. The limits are a ratio (what it has uploaded, over its
//! size), a time spent seeding, or both; with neither it seeds until stopped.
//! Each torrent has its own limits when it asks for them, else the app's.
//!
//! Seeding is a status of its own (`downloads.seeding`), not a download
//! status: a finished torrent stays `Completed`. One task checks every
//! seeding torrent each [`TICK`]: it keeps the torrent live (or pauses it
//! while the network is held), counts what it uploaded and the time it
//! seeded, and stops it when its limits are met. Every change to a seeding
//! torrent takes [`Inner::seeding`], so a check never races a stop, a start
//! or a removal.
//!
//! The engine counts uploads from zero each time a torrent goes live, so the
//! seeder adds up the increase between checks. A pause drops the count, so
//! whatever was uploaded is saved before the pause.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::store::{self, DownloadRow};
use super::{DownloadId, DownloadKind, Event, Inner, Status, torrent_job};
use crate::torrent::{
    ManagedTorrent, TorrentEngine, TorrentOptions, TorrentStats, TorrentStatsState,
    check_seed_limits,
};
use crate::{Error, Result};

/// How often seeding torrents are checked.
pub(super) const TICK: Duration = Duration::from_secs(2);

/// A longer gap between two checks is not counted as seeding time: the
/// machine was asleep, or the app was busy.
const MAX_GAP: Duration = Duration::from_secs(10);

/// The seeding limits torrents without their own use: (ratio, minutes).
pub(super) type SeedDefaults = (Option<f64>, Option<u32>);

/// What the seeder remembers about one torrent between checks.
#[derive(Default)]
pub(super) struct Track {
    /// The engine's upload count at the last check. It starts from zero each
    /// time the torrent goes live, so a reading below this one is a restart.
    counter: u64,
    /// When seeding time was last counted; `None` after a pause or a gap.
    last: Option<Instant>,
    /// Counted time that is not a whole second yet.
    carry: Duration,
}

/// The seeder's per-torrent state. Lives behind [`Inner::seeding`].
#[derive(Default)]
pub(super) struct Tracks(HashMap<DownloadId, Track>);

/// Whether a torrent has met its seeding limits: its upload ratio reaches
/// `ratio_limit`, or it has seeded for `minutes_limit`. A torrent of no size
/// has a ratio of 0. With neither limit it never has.
pub(crate) fn limits_met(
    ratio_limit: Option<f64>,
    minutes_limit: Option<u32>,
    uploaded: u64,
    size: u64,
    seeded_secs: u64,
) -> bool {
    let ratio = if size == 0 {
        0.0
    } else {
        uploaded as f64 / size as f64
    };
    ratio_limit.is_some_and(|limit| ratio >= limit)
        || minutes_limit.is_some_and(|minutes| seeded_secs >= u64::from(minutes) * 60)
}

/// The limits a torrent seeds by: its own when it has custom ones, else the app's.
pub(super) fn limits_for(row: &DownloadRow, defaults: SeedDefaults) -> SeedDefaults {
    match row.torrent_options() {
        Some(TorrentOptions {
            custom_seed: true,
            seed_ratio,
            seed_minutes,
            ..
        }) => (seed_ratio, seed_minutes),
        _ => defaults,
    }
}

fn size_of(row: &DownloadRow) -> u64 {
    row.size.unwrap_or(0).max(0) as u64
}

/// Uploads since the last reading. A reading below the last one means the
/// count started again, so the reading itself is what was uploaded since.
fn take_upload(track: &mut Track, counter: u64) -> u64 {
    let delta = if counter >= track.counter {
        counter - track.counter
    } else {
        counter
    };
    track.counter = counter;
    delta
}

/// Tell the UI where a torrent's seeding stands.
pub(super) fn emit(
    inner: &Inner,
    id: DownloadId,
    uploaded: u64,
    seeded_secs: u64,
    seeding: bool,
    stats: Option<&TorrentStats>,
) {
    let live = stats.and_then(|s| s.live.as_ref());
    inner.emit(Event::Seeding {
        download_id: id,
        uploaded,
        upload_speed: live.map_or(0, |l| l.upload_speed.as_bytes()),
        peers: live.map_or(0, |l| l.snapshot.peer_stats.live as usize),
        seeded_secs,
        seeding,
    });
}

/// Save what a live torrent has uploaded, then pause it in the engine.
/// Returns the upload bytes that were saved now.
async fn save_and_pause(
    inner: &Inner,
    engine: &TorrentEngine,
    id: DownloadId,
    handle: &Arc<ManagedTorrent>,
    track: &mut Track,
) -> Result<u64> {
    let stats = handle.stats();
    let uploaded = match stats.state {
        TorrentStatsState::Live => take_upload(track, stats.uploaded_bytes),
        _ => 0,
    };
    store::add_seeding_progress(&inner.pool, id, uploaded, 0).await?;
    engine.pause(handle).await?;
    track.counter = 0;
    track.last = None;
    Ok(uploaded)
}

/// Seeding stopped (its limits were met, or it was lost): record it.
async fn mark_stopped(inner: &Inner, id: DownloadId, uploaded: u64, secs: u64) -> Result<()> {
    store::set_seeding(&inner.pool, id, false).await?;
    emit(inner, id, uploaded, secs, false, None);
    Ok(())
}

/// The seeder task: checks the seeding torrents every [`TICK`], and at once
/// when told to (a limit changed, or the network hold did). Stops at shutdown.
pub(super) async fn follow(inner: Arc<Inner>) {
    loop {
        if let Err(e) = pass(&inner).await {
            tracing::error!(error = %e, "seeding check failed");
        }
        tokio::select! {
            _ = inner.shutdown.cancelled() => break,
            _ = inner.seed_wake.notified() => {}
            _ = tokio::time::sleep(TICK) => {}
        }
    }
    tracing::debug!("seeder stopped");
}

/// One check of every seeding torrent, under the lock.
pub(super) async fn pass(inner: &Arc<Inner>) -> Result<()> {
    let mut tracks = inner.seeding.lock().await;
    run_pass(inner, &mut tracks).await
}

/// One check of every seeding torrent. The caller holds [`Inner::seeding`].
pub(super) async fn run_pass(inner: &Arc<Inner>, tracks: &mut Tracks) -> Result<()> {
    let Some(engine) = &inner.torrents else {
        return Ok(());
    };
    let rows = store::seeding_downloads(&inner.pool).await?;
    // Torrents that stopped or were removed leave no state behind.
    tracks
        .0
        .retain(|id, _| rows.iter().any(|row| row.id == *id));
    if rows.is_empty() {
        return Ok(());
    }
    let held = inner
        .hold
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    let now = Instant::now();
    for row in rows {
        let track = tracks.0.entry(row.id).or_default();
        match check(inner, engine, &row, track, held, now).await {
            Ok(true) => {}
            Ok(false) => {
                tracks.0.remove(&row.id);
            }
            Err(e) => tracing::warn!(download = row.id, error = %e, "seeding check failed"),
        }
    }
    Ok(())
}

/// Check one seeding torrent. Returns whether it is still seeding.
async fn check(
    inner: &Inner,
    engine: &TorrentEngine,
    row: &DownloadRow,
    track: &mut Track,
    held: bool,
    now: Instant,
) -> Result<bool> {
    let id = row.id;
    let mut uploaded = row.uploaded as u64;
    let mut secs = row.seeded_secs as u64;
    let Some(info_hash) = row.info_hash.as_deref() else {
        // Without an info hash there was never any metadata to seed from.
        mark_stopped(inner, id, uploaded, secs).await?;
        return Ok(false);
    };

    if held {
        // The network is held: stop uploading, but stay seeding, and count
        // nothing until it is released. Only a session that is already
        // running has the torrent; a held app doesn't start one.
        if let Some(handle) = engine.running_handle(info_hash) {
            uploaded += save_and_pause(inner, engine, id, &handle, track).await?;
        }
        track.last = None;
        emit(inner, id, uploaded, secs, true, None);
        return Ok(true);
    }

    let handle = match engine.find(info_hash).await? {
        Some(handle) => handle,
        None => match torrent_job::readd(engine, row).await? {
            Some(handle) => {
                track.counter = 0;
                track.last = None;
                handle
            }
            None => {
                tracing::warn!(download = id, "torrent metadata is gone; seeding stopped");
                mark_stopped(inner, id, uploaded, secs).await?;
                return Ok(false);
            }
        },
    };
    if handle.is_paused() {
        engine.resume(&handle).await?;
        track.counter = 0;
        track.last = None;
    }

    let stats = handle.stats();
    match stats.state {
        TorrentStatsState::Live if !stats.finished => {
            // Its data isn't all there any more (files moved or deleted
            // outside the app): seeding only ever uploads, so it stops
            // rather than quietly downloading the torrent again.
            tracing::warn!(download = id, "torrent data incomplete; seeding stopped");
            uploaded += save_and_pause(inner, engine, id, &handle, track).await?;
            track.last = None;
            mark_stopped(inner, id, uploaded, secs).await?;
            Ok(false)
        }
        TorrentStatsState::Live => {
            let up = take_upload(track, stats.uploaded_bytes);
            if let Some(last) = track.last {
                track.carry += now.saturating_duration_since(last).min(MAX_GAP);
            }
            track.last = Some(now);
            let whole = track.carry.as_secs();
            track.carry -= Duration::from_secs(whole);
            uploaded += up;
            secs += whole;
            store::add_seeding_progress(&inner.pool, id, up, whole).await?;

            let (ratio, minutes) = limits_for(row, inner.seed_defaults());
            if limits_met(ratio, minutes, uploaded, size_of(row), secs) {
                engine.pause(&handle).await?;
                track.counter = 0;
                track.last = None;
                mark_stopped(inner, id, uploaded, secs).await?;
                return Ok(false);
            }
            emit(inner, id, uploaded, secs, true, Some(&stats));
            Ok(true)
        }
        TorrentStatsState::Error => {
            tracing::warn!(
                download = id,
                error = ?stats.error,
                "torrent failed while seeding; adding it again"
            );
            engine.forget(info_hash, false).await?;
            track.counter = 0;
            track.last = None;
            emit(inner, id, uploaded, secs, true, None);
            Ok(true)
        }
        // Starting up (or paused by someone else): nothing to count yet.
        _ => {
            track.counter = 0;
            track.last = None;
            emit(inner, id, uploaded, secs, true, Some(&stats));
            Ok(true)
        }
    }
}

/// Save what the seeding torrents have uploaded since the last check, without
/// pausing them. Used before the session stops, so an app exit loses at most
/// the time since the last check.
pub(super) async fn save_uploads(inner: &Arc<Inner>, tracks: &mut Tracks) -> Result<()> {
    let Some(engine) = &inner.torrents else {
        return Ok(());
    };
    for row in store::seeding_downloads(&inner.pool).await? {
        let Some(handle) = row
            .info_hash
            .as_deref()
            .and_then(|hash| engine.running_handle(hash))
        else {
            continue;
        };
        let stats = handle.stats();
        if let TorrentStatsState::Live = stats.state {
            let track = tracks.0.entry(row.id).or_default();
            let up = take_upload(track, stats.uploaded_bytes);
            store::add_seeding_progress(&inner.pool, row.id, up, 0).await?;
        }
    }
    Ok(())
}

/// Stop a torrent seeding by hand: it stops uploading and keeps its files.
/// A torrent that isn't seeding is left alone.
pub(super) async fn stop_by_hand(
    inner: &Arc<Inner>,
    tracks: &mut Tracks,
    id: DownloadId,
) -> Result<()> {
    let row = store::get_download(&inner.pool, id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("download {id}")))?;
    if row.kind() != DownloadKind::Torrent {
        return Err(Error::InvalidInput("Only a torrent can seed.".into()));
    }
    if row.seeding == 0 {
        return Ok(());
    }
    let mut uploaded = row.uploaded as u64;
    let secs = row.seeded_secs as u64;
    if let (Some(engine), Some(info_hash)) = (&inner.torrents, row.info_hash.as_deref())
        && let Some(handle) = engine.find(info_hash).await?
    {
        // The track is only dropped once the upload count is saved.
        let track = tracks.0.entry(id).or_default();
        uploaded += save_and_pause(inner, engine, id, &handle, track).await?;
    }
    tracks.0.remove(&id);
    mark_stopped(inner, id, uploaded, secs).await?;
    tracing::info!(download = id, "seeding stopped by hand");
    Ok(())
}

/// Start seeding a finished torrent again. If its limits are already met, it
/// is given limits of its own that are none, so it seeds until stopped.
pub(super) async fn start_by_hand(
    inner: &Arc<Inner>,
    tracks: &mut Tracks,
    id: DownloadId,
) -> Result<()> {
    let row = store::get_download(&inner.pool, id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("download {id}")))?;
    if row.kind() != DownloadKind::Torrent {
        return Err(Error::InvalidInput("Only a torrent can seed.".into()));
    }
    if row.status() != Status::Completed {
        return Err(Error::InvalidInput(
            "Only a finished torrent can seed.".into(),
        ));
    }
    if row.seeding != 0 {
        return Ok(());
    }
    let (ratio, minutes) = limits_for(&row, inner.seed_defaults());
    if limits_met(
        ratio,
        minutes,
        row.uploaded as u64,
        size_of(&row),
        row.seeded_secs as u64,
    ) {
        let mut options = row.torrent_options().unwrap_or_default();
        options.custom_seed = true;
        options.seed_ratio = None;
        options.seed_minutes = None;
        store::set_torrent_options(&inner.pool, id, &options).await?;
        tracing::info!(
            download = id,
            "seeding without limits, as its limits are met"
        );
    }
    store::set_seeding(&inner.pool, id, true).await?;
    // Stopped torrents have no upload count to keep: start from zero.
    tracks.0.remove(&id);
    run_pass(inner, tracks).await
}

/// Give a torrent its own seeding limits (`custom`), or none of its own. The
/// limits are checked against the torrent at once.
pub(super) async fn set_limits(
    inner: &Arc<Inner>,
    tracks: &mut Tracks,
    id: DownloadId,
    custom: bool,
    ratio: Option<f64>,
    minutes: Option<u32>,
) -> Result<()> {
    check_seed_limits(ratio, minutes)?;
    let row = store::get_download(&inner.pool, id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("download {id}")))?;
    if row.kind() != DownloadKind::Torrent {
        return Err(Error::InvalidInput(
            "Only a torrent has seeding limits.".into(),
        ));
    }
    let mut options = row.torrent_options().unwrap_or_default();
    options.custom_seed = custom;
    options.seed_ratio = ratio;
    options.seed_minutes = minutes;
    store::set_torrent_options(&inner.pool, id, &options).await?;
    run_pass(inner, tracks).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ratio_limit_is_met_at_the_ratio() {
        assert!(!limits_met(Some(1.0), None, 99, 100, 0));
        assert!(limits_met(Some(1.0), None, 100, 100, 0));
        assert!(limits_met(Some(0.5), None, 60, 100, 0));
        // A torrent of no size has no ratio, so no ratio limit is ever met.
        assert!(!limits_met(Some(1.0), None, 5, 0, 0));
    }

    #[test]
    fn a_time_limit_is_met_at_the_minute() {
        assert!(!limits_met(None, Some(2), 0, 100, 119));
        assert!(limits_met(None, Some(2), 0, 100, 120));
    }

    #[test]
    fn either_limit_is_enough_and_no_limits_never_stop() {
        assert!(limits_met(Some(2.0), Some(1), 100, 100, 60));
        assert!(!limits_met(None, None, u64::MAX, 1, u64::MAX));
    }

    #[test]
    fn upload_counts_survive_a_restart_of_the_counter() {
        let mut track = Track::default();
        assert_eq!(take_upload(&mut track, 100), 100);
        assert_eq!(take_upload(&mut track, 250), 150);
        // The torrent went live again: its count began at 40.
        assert_eq!(take_upload(&mut track, 40), 40);
        assert_eq!(take_upload(&mut track, 60), 20);
    }
}
