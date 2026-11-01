//! Checksums of finished downloads: the file is checked against the value
//! the user set, or one a checksum list (`.sfv`, `.md5`, ...) downloaded into
//! the same package supplied. A mismatch fails the download and keeps the
//! file; retrying deletes it and downloads again.

use std::path::Path;
use std::sync::Arc;

use percent_encoding::percent_decode_str;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::store::{self, DownloadRow};
use super::{DownloadId, DownloadKind, Event, Inner, PackageId, Status};
use crate::checksum::{self, Checksum, ChecksumAlgo};
use crate::{Error, Result};

/// Checksum lists bigger than this aren't read.
const MAX_LIST: u64 = 1024 * 1024;

/// Set or clear a download's expected checksum. A file that's already
/// there (completed, or failed on its checksum) is checked again in the
/// background; one still downloading is checked when it finishes.
pub(super) async fn set(
    inner: &Arc<Inner>,
    row: &DownloadRow,
    checksum: Option<(ChecksumAlgo, &str)>,
) -> Result<()> {
    let failed_on_checksum = row.status() == Status::Failed && row.checksum_ok == Some(0);
    store::set_checksum(&inner.pool, row.id, checksum).await?;
    tracing::info!(
        download = row.id,
        algo = checksum.map(|(a, _)| a.as_str()),
        "checksum set"
    );
    inner.emit(Event::ChecksumChanged {
        download_id: row.id,
        checksum: checksum.map(|(algo, value)| Checksum {
            algo,
            value: value.to_owned(),
            verified: None,
        }),
    });
    let file_there = row.status() == Status::Completed || failed_on_checksum;
    if checksum.is_some() && file_there && !inner.running().contains_key(&row.id) {
        let inner = Arc::clone(inner);
        let id = row.id;
        tokio::spawn(async move {
            if let Err(e) = recheck(&inner, id).await {
                tracing::warn!(download = id, error = %e, "could not check checksum");
            }
        });
    }
    Ok(())
}

/// Check a download that has just written `path`. Passes when it has no
/// expected checksum, or when `cancel` fires before the check is done.
pub(super) async fn check_finished(
    inner: &Inner,
    id: DownloadId,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<()> {
    let Some(row) = store::get_download(&inner.pool, id).await? else {
        return Ok(());
    };
    match check(inner, &row, path, cancel).await? {
        Some(mismatch) => Err(mismatch),
        None => Ok(()),
    }
}

/// Hash `path`, record and announce whether it matched. Returns the
/// mismatch; `None` if it matched or there was nothing to compare.
async fn check(
    inner: &Inner,
    row: &DownloadRow,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<Option<Error>> {
    let Some(expected) = row.checksum() else {
        return Ok(None);
    };
    tracing::info!(
        download = row.id,
        algo = expected.algo.as_str(),
        "checking checksum"
    );
    let Some(actual) = checksum::of_file(expected.algo, path, cancel).await? else {
        return Ok(None);
    };
    let ok = actual == expected.value;
    // A value changed while hashing makes this check stale.
    if !store::set_checksum_ok(&inner.pool, row.id, &expected.value, Some(ok)).await? {
        return Ok(None);
    }
    inner.emit(Event::ChecksumChanged {
        download_id: row.id,
        checksum: Some(Checksum {
            verified: Some(ok),
            ..expected.clone()
        }),
    });
    if ok {
        tracing::info!(download = row.id, "checksum matches");
        return Ok(None);
    }
    Ok(Some(Error::ChecksumMismatch {
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        expected: expected.value,
        actual,
    }))
}

/// Check a file that is already there. A completed download that doesn't
/// match fails; one that failed on its checksum and now matches completes.
async fn recheck(inner: &Inner, id: DownloadId) -> Result<()> {
    let Some(row) = store::get_download(&inner.pool, id).await? else {
        return Ok(());
    };
    let Some(path) = row.dest_path.as_deref().map(Path::new) else {
        return Ok(());
    };
    if !tokio::fs::try_exists(path).await? {
        tracing::info!(download = id, "file is gone; checksum not checked");
        return Ok(());
    }
    let mismatch = check(inner, &row, path, &inner.shutdown).await?;
    let verified = store::get_download(&inner.pool, id)
        .await?
        .and_then(|r| r.checksum_ok);
    let pool = &inner.pool;
    let (status, error) = match (row.status(), mismatch) {
        (Status::Completed, Some(mismatch)) => {
            let message = mismatch.to_string();
            tracing::warn!(download = id, error = %message, "completed download doesn't match");
            store::set_status(pool, id, Status::Failed, Some(&message)).await?;
            (Status::Failed, Some(message))
        }
        (Status::Failed, None) if verified == Some(1) => {
            let size = tokio::fs::metadata(path).await?.len();
            store::mark_completed(pool, id, size).await?;
            super::job::record_history(inner, &row, path, size).await;
            (Status::Completed, None)
        }
        _ => return Ok(()),
    };
    inner.emit(Event::StatusChanged {
        download_id: id,
        status,
        error,
    });
    Ok(())
}

/// When the file a download just finished is a checksum list, give the
/// files of its package that it names (and that have no checksum yet)
/// their expected values.
pub(super) async fn apply_list(
    inner: &Arc<Inner>,
    package: PackageId,
    list_id: DownloadId,
    path: &Path,
) -> Result<()> {
    let Some(algo) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(ChecksumAlgo::for_list)
    else {
        return Ok(());
    };
    if tokio::fs::metadata(path).await?.len() > MAX_LIST {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&tokio::fs::read(path).await?).into_owned();
    let entries = checksum::parse_list(algo, &text);
    let mut applied = 0;
    for row in store::package_downloads(&inner.pool, package).await? {
        if row.id == list_id || row.kind() == DownloadKind::Gallerydl || row.checksum_algo.is_some()
        {
            continue;
        }
        let names = names_of(&row);
        let entry = entries
            .iter()
            .find(|e| names.iter().any(|n| n.eq_ignore_ascii_case(&e.name)));
        if let Some(entry) = entry {
            set(inner, &row, Some((algo, &entry.value))).await?;
            applied += 1;
        }
    }
    tracing::info!(
        list = %path.display(),
        entries = entries.len(),
        applied,
        "checksum list read"
    );
    Ok(())
}

/// Names a download's file may go by: where it was saved, the name it was
/// given, and the last part of its link.
fn names_of(row: &DownloadRow) -> Vec<String> {
    let saved = row
        .dest_path
        .as_deref()
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let from_url = Url::parse(&row.source_url).ok().and_then(|url| {
        let last = url.path_segments()?.next_back()?.to_owned();
        Some(percent_decode_str(&last).decode_utf8_lossy().into_owned())
    });
    [saved, row.filename.clone(), from_url]
        .into_iter()
        .flatten()
        .filter(|n| !n.is_empty())
        .collect()
}
