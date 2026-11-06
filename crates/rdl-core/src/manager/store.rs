//! SQL for the manager. Byte counts are `u64` in Rust and `INTEGER` (i64) in
//! SQLite; files above 8 EiB are not a concern.

use std::path::{Path, PathBuf};

use sqlx::AssertSqlSafe;

use super::queues::{OnComplete, QueueChanges, QueueSchedule};
use super::types::{
    Download, DownloadId, DownloadKind, FileCount, NewDownload, NewPackage, Package, PackageId,
    QueueId, Status,
};
use crate::Result;
use crate::checksum::{Checksum, ChecksumAlgo};
use crate::db::Pool;
use crate::http::Segment;
use crate::proxy::ProxyChoice;
use crate::request::RequestContext;

/// A download row with the fields the job runner needs beyond [`Download`].
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct DownloadRow {
    pub id: i64,
    pub package_id: i64,
    pub kind: String,
    pub source_url: String,
    pub filename: Option<String>,
    pub dest_path: Option<String>,
    pub size: Option<i64>,
    pub downloaded: i64,
    pub status: String,
    pub error: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub handler_opts_json: Option<String>,
    pub title: Option<String>,
    pub files_done: Option<i64>,
    pub files_total: Option<i64>,
    pub request_json: Option<String>,
    pub speed_limit: Option<i64>,
    pub checksum_algo: Option<String>,
    pub checksum_value: Option<String>,
    pub checksum_ok: Option<i64>,
    pub proxy: Option<String>,
}

impl DownloadRow {
    pub fn proxy(&self) -> ProxyChoice {
        ProxyChoice::from_db(self.proxy.as_deref())
    }

    /// The expected checksum, if one was set.
    pub fn checksum(&self) -> Option<Checksum> {
        Some(Checksum {
            algo: ChecksumAlgo::parse(self.checksum_algo.as_deref()?)?,
            value: self.checksum_value.clone()?,
            verified: self.checksum_ok.map(|ok| ok != 0),
        })
    }

    pub fn speed_limit(&self) -> Option<u64> {
        self.speed_limit.map(|l| l as u64)
    }

    pub fn kind(&self) -> DownloadKind {
        DownloadKind::parse(&self.kind)
    }

    /// Handler options of `kind`; a row with unreadable options gets the defaults.
    fn options<T: serde::de::DeserializeOwned + Default>(&self, kind: DownloadKind) -> Option<T> {
        (self.kind() == kind).then(|| {
            self.handler_opts_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default()
        })
    }

    pub fn ytdlp_options(&self) -> Option<crate::ytdlp::YtDlpOptions> {
        self.options(DownloadKind::Ytdlp)
    }

    pub fn gallerydl_options(&self) -> Option<crate::gallerydl::GalleryDlOptions> {
        self.options(DownloadKind::Gallerydl)
    }

    /// Referer, user agent and cookies to send.
    pub fn request(&self) -> RequestContext {
        RequestContext::from_json(self.request_json.as_deref())
    }

    pub fn files(&self) -> Option<FileCount> {
        (self.kind() == DownloadKind::Gallerydl).then(|| FileCount {
            done: self.files_done.unwrap_or(0) as u32,
            total: self.files_total.map(|t| t as u32),
        })
    }

    pub fn status(&self) -> Status {
        // The CHECK constraint guarantees a known value.
        Status::parse(&self.status).unwrap_or(Status::Failed)
    }

    pub fn to_download(&self) -> Download {
        Download {
            id: self.id,
            package_id: self.package_id,
            kind: self.kind.clone(),
            source_url: self.source_url.clone(),
            filename: self.filename.clone(),
            dest_path: self.dest_path.as_ref().map(PathBuf::from),
            size: self.size.map(|s| s as u64),
            downloaded: self.downloaded as u64,
            status: self.status(),
            error: self.error.clone(),
            ytdlp: self.ytdlp_options(),
            gallerydl: self.gallerydl_options(),
            files: self.files(),
            title: self.title.clone(),
            speed_limit: self.speed_limit(),
            checksum: self.checksum(),
            proxy: self.proxy(),
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PackageRow {
    pub id: i64,
    pub name: String,
    pub dest_dir: String,
    pub queue_id: i64,
    pub priority: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct QueueRow {
    pub id: i64,
    pub name: String,
    pub max_active: i64,
    pub schedule_json: Option<String>,
    pub on_complete_action: String,
}

impl QueueRow {
    /// The schedule; unreadable JSON counts as none.
    pub fn schedule(&self) -> QueueSchedule {
        self.schedule_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default()
    }

    pub fn on_complete(&self) -> OnComplete {
        OnComplete::parse(&self.on_complete_action)
    }
}

/// Packages and downloads per state in one queue.
#[derive(Debug, Clone, Copy, Default, sqlx::FromRow)]
pub(crate) struct QueueCounts {
    pub queue_id: i64,
    pub packages: i64,
    pub active: i64,
    pub queued: i64,
    pub paused: i64,
}

/// Facts learned from the first probe, stored so resume uses the same file.
pub(crate) struct Resolved<'a> {
    pub resolved_url: &'a str,
    pub filename: &'a str,
    pub dest_path: &'a Path,
    pub size: Option<u64>,
    pub etag: Option<&'a str>,
    pub last_modified: Option<&'a str>,
}

/// SQL `IN (...)` list for statuses. Built only from [`Status::as_str`]
/// constants, never from input, which is why callers may use `AssertSqlSafe`.
fn status_list(statuses: &[Status]) -> String {
    statuses
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) async fn insert_package(pool: &Pool, new: &NewPackage) -> Result<PackageId> {
    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO packages (name, dest_dir, queue_id, position)
         VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position) + 1, 0) FROM packages WHERE queue_id = ?3))
         RETURNING id",
    )
    .bind(&new.name)
    .bind(new.dest_dir.to_string_lossy().into_owned())
    .bind(new.queue_id)
    .fetch_one(&mut *tx)
    .await?;

    insert_downloads(&mut tx, id, &new.downloads, 0).await?;
    tx.commit().await?;
    Ok(id)
}

/// Insert `downloads` into package `package`, numbered from `first`.
async fn insert_downloads(
    conn: &mut sqlx::SqliteConnection,
    package: i64,
    downloads: &[NewDownload],
    first: i64,
) -> Result<()> {
    for (offset, download) in downloads.iter().enumerate() {
        let opts = match (&download.ytdlp, &download.gallerydl) {
            (Some(o), _) => Some(serde_json::to_string(o)?),
            (None, Some(o)) => Some(serde_json::to_string(o)?),
            (None, None) => None,
        };
        sqlx::query(
            "INSERT INTO downloads (package_id, kind, source_url, filename, handler_opts_json, title,
                 files_total, size, request_json, proxy, position)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(package)
        .bind(download.kind.as_str())
        .bind(download.url.as_str())
        .bind(download.filename.as_deref())
        .bind(opts)
        .bind(download.title.as_deref())
        .bind(download.files_total.map(i64::from))
        .bind(download.size.map(|s| s as i64))
        .bind(download.request.clone().cleaned().to_json()?)
        .bind(download.proxy.to_db())
        .bind(first + offset as i64)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Add `downloads` to the end of an existing package.
pub(crate) async fn add_downloads(
    pool: &Pool,
    package: PackageId,
    downloads: &[NewDownload],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let first: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM downloads WHERE package_id = ?",
    )
    .bind(package)
    .fetch_one(&mut *tx)
    .await?;
    insert_downloads(&mut tx, package, downloads, first).await?;
    tx.commit().await?;
    Ok(())
}

/// Move a download that hasn't started into `package`, at its end. A running
/// one is left alone (returns false), so its files never move under it.
pub(crate) async fn move_download(pool: &Pool, id: DownloadId, package: PackageId) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE downloads SET package_id = ?1,
             position = (SELECT COALESCE(MAX(position) + 1, 0) FROM downloads WHERE package_id = ?1)
         WHERE id = ?2 AND status != 'active'",
    )
    .bind(package)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn get_download(pool: &Pool, id: DownloadId) -> Result<Option<DownloadRow>> {
    Ok(sqlx::query_as("SELECT * FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub(crate) async fn get_package(pool: &Pool, id: PackageId) -> Result<Option<PackageRow>> {
    Ok(sqlx::query_as("SELECT * FROM packages WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub(crate) async fn list_packages(pool: &Pool) -> Result<Vec<Package>> {
    let packages: Vec<PackageRow> =
        sqlx::query_as("SELECT * FROM packages ORDER BY queue_id, priority DESC, position, id")
            .fetch_all(pool)
            .await?;
    let downloads: Vec<DownloadRow> =
        sqlx::query_as("SELECT * FROM downloads ORDER BY package_id, position, id")
            .fetch_all(pool)
            .await?;

    Ok(packages
        .into_iter()
        .map(|p| Package {
            downloads: downloads
                .iter()
                .filter(|d| d.package_id == p.id)
                .map(DownloadRow::to_download)
                .collect(),
            id: p.id,
            name: p.name,
            dest_dir: PathBuf::from(p.dest_dir),
            queue_id: p.queue_id,
            priority: p.priority,
        })
        .collect())
}

/// A queue's packages in the order they start (after priority).
pub(crate) async fn queue_package_ids(pool: &Pool, queue: QueueId) -> Result<Vec<PackageId>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM packages WHERE queue_id = ? ORDER BY priority DESC, position, id",
    )
    .bind(queue)
    .fetch_all(pool)
    .await?)
}

/// Number the packages in the given order.
pub(crate) async fn set_package_order(pool: &Pool, ids: &[PackageId]) -> Result<()> {
    let mut tx = pool.begin().await?;
    for (position, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE packages SET position = ? WHERE id = ?")
            .bind(position as i64)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn queues(pool: &Pool) -> Result<Vec<QueueRow>> {
    Ok(sqlx::query_as(
        "SELECT id, name, max_active, schedule_json, on_complete_action FROM queues ORDER BY position, id",
    )
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn queue(pool: &Pool, id: QueueId) -> Result<Option<QueueRow>> {
    Ok(sqlx::query_as(
        "SELECT id, name, max_active, schedule_json, on_complete_action FROM queues WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn queue_counts(pool: &Pool) -> Result<Vec<QueueCounts>> {
    Ok(sqlx::query_as(
        "SELECT p.queue_id AS queue_id,
                COUNT(DISTINCT p.id) AS packages,
                COALESCE(SUM(d.status = 'active'), 0) AS active,
                COALESCE(SUM(d.status = 'queued'), 0) AS queued,
                COALESCE(SUM(d.status = 'paused'), 0) AS paused
         FROM packages p LEFT JOIN downloads d ON d.package_id = p.id
         GROUP BY p.queue_id",
    )
    .fetch_all(pool)
    .await?)
}

/// Downloads in the queue that still have something to do (running,
/// waiting, or paused by the user).
pub(crate) async fn pending_in_queue(pool: &Pool, queue: QueueId) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM downloads d JOIN packages p ON p.id = d.package_id
         WHERE p.queue_id = ? AND d.status IN ('active', 'queued', 'paused')",
    )
    .bind(queue)
    .fetch_one(pool)
    .await?)
}

/// A clash with an existing queue's name, as a message for the user.
fn name_taken(e: sqlx::Error, name: &str) -> crate::Error {
    if e.as_database_error()
        .is_some_and(|d| d.is_unique_violation())
    {
        crate::Error::InvalidInput(format!("There is already a queue called \"{name}\"."))
    } else {
        e.into()
    }
}

pub(crate) async fn insert_queue(pool: &Pool, name: &str, max_active: u32) -> Result<QueueId> {
    sqlx::query_scalar(
        "INSERT INTO queues (name, max_active, position)
         VALUES (?, ?, (SELECT COALESCE(MAX(position) + 1, 0) FROM queues))
         RETURNING id",
    )
    .bind(name)
    .bind(i64::from(max_active))
    .fetch_one(pool)
    .await
    .map_err(|e| name_taken(e, name))
}

/// Returns whether the queue exists.
pub(crate) async fn update_queue(pool: &Pool, id: QueueId, c: &QueueChanges) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE queues SET name = ?, max_active = ?, schedule_json = ?, on_complete_action = ?
         WHERE id = ?",
    )
    .bind(&c.name)
    .bind(i64::from(c.max_active))
    .bind(serde_json::to_string(&c.schedule)?)
    .bind(c.on_complete.as_str())
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| name_taken(e, &c.name))?;
    Ok(result.rows_affected() > 0)
}

/// Move a queue's packages to `into` (after its own), then delete it.
pub(crate) async fn delete_queue(pool: &Pool, id: QueueId, into: QueueId) -> Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE packages SET queue_id = ?1,
             position = position + (SELECT COALESCE(MAX(position) + 1, 0) FROM packages WHERE queue_id = ?1)
         WHERE queue_id = ?2",
    )
    .bind(into)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let result = sqlx::query("DELETE FROM queues WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(result.rows_affected() > 0)
}

/// Put a package at the end of another queue.
pub(crate) async fn set_package_queue(
    pool: &Pool,
    package: PackageId,
    queue: QueueId,
) -> Result<()> {
    sqlx::query(
        "UPDATE packages SET queue_id = ?1,
             position = (SELECT COALESCE(MAX(position) + 1, 0) FROM packages WHERE queue_id = ?1)
         WHERE id = ?2",
    )
    .bind(queue)
    .bind(package)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn set_queue_limit(pool: &Pool, queue: QueueId, max_active: u32) -> Result<bool> {
    let result = sqlx::query("UPDATE queues SET max_active = ? WHERE id = ?")
        .bind(i64::from(max_active.max(1)))
        .bind(queue)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Next downloads to start in `queue`: package priority first, then the order
/// packages and their downloads were added.
pub(crate) async fn next_queued(
    pool: &Pool,
    queue: QueueId,
    limit: usize,
) -> Result<Vec<DownloadRow>> {
    Ok(sqlx::query_as(
        "SELECT d.* FROM downloads d JOIN packages p ON p.id = d.package_id
         WHERE p.queue_id = ? AND d.status = 'queued'
         ORDER BY p.priority DESC, p.position, p.id, d.position, d.id
         LIMIT ?",
    )
    .bind(queue)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn ids_with_status(pool: &Pool, statuses: &[Status]) -> Result<Vec<DownloadId>> {
    let sql = format!(
        "SELECT id FROM downloads WHERE status IN ({}) ORDER BY id",
        status_list(statuses)
    );
    Ok(sqlx::query_scalar(AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn package_download_ids(
    pool: &Pool,
    package: PackageId,
) -> Result<Vec<DownloadId>> {
    Ok(
        sqlx::query_scalar("SELECT id FROM downloads WHERE package_id = ? ORDER BY id")
            .bind(package)
            .fetch_all(pool)
            .await?,
    )
}

/// Anything left `active` was interrupted by a crash or kill; queue it again.
pub(crate) async fn requeue_interrupted(pool: &Pool) -> Result<u64> {
    Ok(
        sqlx::query("UPDATE downloads SET status = 'queued' WHERE status = 'active'")
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// Queued → active; returns false if it isn't queued any more (paused or
/// removed meanwhile).
pub(crate) async fn mark_active(pool: &Pool, id: DownloadId) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE downloads SET status = 'active', error = NULL,
             started_at = COALESCE(started_at, CAST(unixepoch('subsec') * 1000 AS INTEGER))
         WHERE id = ? AND status = 'queued'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn set_status(
    pool: &Pool,
    id: DownloadId,
    status: Status,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE downloads SET status = ?, error = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Move to `to` only from one of `from`; returns whether the row changed.
pub(crate) async fn transition(
    pool: &Pool,
    id: DownloadId,
    from: &[Status],
    to: Status,
) -> Result<bool> {
    let sql = format!(
        "UPDATE downloads SET status = ?, error = NULL WHERE id = ? AND status IN ({})",
        status_list(from)
    );
    let result = sqlx::query(AssertSqlSafe(sql))
        .bind(to.as_str())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Queue a completed gallery download again. Its folder and files stay; the
/// counts start over and the total is unknown until the run finishes.
/// Returns whether the row was a completed gallery.
pub(crate) async fn restart_gallery(pool: &Pool, id: DownloadId) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE downloads SET status = 'queued', error = NULL, files_done = 0,
             files_total = NULL, downloaded = 0, size = NULL, finished_at = NULL
         WHERE id = ? AND status = 'completed' AND kind = 'gallerydl'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn set_resolved(pool: &Pool, id: DownloadId, r: &Resolved<'_>) -> Result<()> {
    sqlx::query(
        "UPDATE downloads SET resolved_url = ?, filename = ?, dest_path = ?, size = ?, etag = ?, last_modified = ?
         WHERE id = ?",
    )
    .bind(r.resolved_url)
    .bind(r.filename)
    .bind(r.dest_path.to_string_lossy().into_owned())
    .bind(r.size.map(|s| s as i64))
    .bind(r.etag)
    .bind(r.last_modified)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Where yt-dlp said it will write. `filename` is not touched: it holds the
/// name the user chose, which must survive into the next run.
pub(crate) async fn set_planned(
    pool: &Pool,
    id: DownloadId,
    dest_path: &Path,
    size: Option<u64>,
) -> Result<()> {
    sqlx::query("UPDATE downloads SET dest_path = ?, size = COALESCE(?, size) WHERE id = ?")
        .bind(dest_path.to_string_lossy().into_owned())
        .bind(size.map(|s| s as i64))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Returns whether the download exists.
pub(crate) async fn set_speed_limit(
    pool: &Pool,
    id: DownloadId,
    limit: Option<u64>,
) -> Result<bool> {
    let result = sqlx::query("UPDATE downloads SET speed_limit = ? WHERE id = ?")
        .bind(limit.map(|l| l as i64))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Set or clear the expected checksum, forgetting any earlier check.
/// Returns whether the download exists.
pub(crate) async fn set_checksum(
    pool: &Pool,
    id: DownloadId,
    checksum: Option<(ChecksumAlgo, &str)>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE downloads SET checksum_algo = ?, checksum_value = ?, checksum_ok = NULL WHERE id = ?",
    )
    .bind(checksum.map(|(algo, _)| algo.as_str()))
    .bind(checksum.map(|(_, value)| value))
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Record whether the file matched `value`, unless the expected value has
/// changed meanwhile. Returns whether it was recorded.
pub(crate) async fn set_checksum_ok(
    pool: &Pool,
    id: DownloadId,
    value: &str,
    ok: Option<bool>,
) -> Result<bool> {
    let result =
        sqlx::query("UPDATE downloads SET checksum_ok = ? WHERE id = ? AND checksum_value = ?")
            .bind(ok.map(i64::from))
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// Returns whether the download exists.
pub(crate) async fn set_proxy(pool: &Pool, id: DownloadId, proxy: ProxyChoice) -> Result<bool> {
    let result = sqlx::query("UPDATE downloads SET proxy = ? WHERE id = ?")
        .bind(proxy.to_db())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Every download in a package, in order.
pub(crate) async fn package_downloads(pool: &Pool, package: PackageId) -> Result<Vec<DownloadRow>> {
    Ok(
        sqlx::query_as("SELECT * FROM downloads WHERE package_id = ? ORDER BY position, id")
            .bind(package)
            .fetch_all(pool)
            .await?,
    )
}

pub(crate) async fn set_downloaded(pool: &Pool, id: DownloadId, downloaded: u64) -> Result<()> {
    sqlx::query("UPDATE downloads SET downloaded = ? WHERE id = ?")
        .bind(downloaded as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Progress of a gallery download: files done and their bytes.
pub(crate) async fn set_files_done(
    pool: &Pool,
    id: DownloadId,
    files: u32,
    downloaded: u64,
) -> Result<()> {
    sqlx::query("UPDATE downloads SET files_done = ?, downloaded = ? WHERE id = ?")
        .bind(i64::from(files))
        .bind(downloaded as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn set_files_total(pool: &Pool, id: DownloadId, total: u32) -> Result<()> {
    sqlx::query("UPDATE downloads SET files_total = ? WHERE id = ?")
        .bind(i64::from(total))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remember a file a gallery download wrote.
pub(crate) async fn add_file(pool: &Pool, id: DownloadId, path: &Path, size: u64) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO download_files (download_id, path, size) VALUES (?, ?, ?)")
        .bind(id)
        .bind(path.to_string_lossy().into_owned())
        .bind(size as i64)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn files_of(pool: &Pool, id: DownloadId) -> Result<Vec<PathBuf>> {
    let paths: Vec<String> =
        sqlx::query_scalar("SELECT path FROM download_files WHERE download_id = ? ORDER BY path")
            .bind(id)
            .fetch_all(pool)
            .await?;
    Ok(paths.into_iter().map(PathBuf::from).collect())
}

/// Whether another download already claimed `path` (case-insensitive, as on NTFS).
pub(crate) async fn dest_taken(pool: &Pool, path: &Path) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM downloads WHERE dest_path = ? COLLATE NOCASE)",
    )
    .bind(path.to_string_lossy().into_owned())
    .fetch_one(pool)
    .await?)
}

pub(crate) async fn mark_completed(pool: &Pool, id: DownloadId, size: u64) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM segments WHERE download_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE downloads SET status = 'completed', error = NULL, size = ?1, downloaded = ?1,
             request_json = NULL, finished_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
         WHERE id = ?2",
    )
    .bind(size as i64)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Replace the stored segment state (and the `downloaded` total) atomically.
pub(crate) async fn save_segments(pool: &Pool, id: DownloadId, segments: &[Segment]) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM segments WHERE download_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for (idx, s) in segments.iter().enumerate() {
        sqlx::query(
            "INSERT INTO segments (download_id, idx, start_byte, end_byte, written) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(idx as i64)
        .bind(s.start as i64)
        .bind(s.end as i64)
        .bind(s.written as i64)
        .execute(&mut *tx)
        .await?;
    }
    let downloaded: u64 = segments.iter().map(|s| s.written).sum();
    sqlx::query("UPDATE downloads SET downloaded = ? WHERE id = ?")
        .bind(downloaded as i64)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn load_segments(pool: &Pool, id: DownloadId) -> Result<Vec<Segment>> {
    let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT start_byte, end_byte, written FROM segments WHERE download_id = ? ORDER BY idx",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(start, end, written)| Segment {
            start: start as u64,
            end: end as u64,
            written: written as u64,
        })
        .collect())
}

pub(crate) async fn clear_segments(pool: &Pool, id: DownloadId) -> Result<()> {
    save_segments(pool, id, &[]).await
}

pub(crate) async fn delete_download(pool: &Pool, id: DownloadId) -> Result<()> {
    sqlx::query("DELETE FROM downloads WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete the package if it has no downloads left; returns whether it did.
pub(crate) async fn delete_package_if_empty(pool: &Pool, id: PackageId) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM packages WHERE id = ? AND NOT EXISTS (SELECT 1 FROM downloads WHERE package_id = packages.id)",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}
