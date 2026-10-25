//! Download history: one row per finished download, looked up by source URL
//! to warn before downloading the same thing twice.
//!
//! Rows are keyed by a SHA-256 of the URL (fragment dropped), not by download
//! id, because SQLite reuses ids once a download is removed. Whether the file
//! is still on disk is checked when rows are read, never stored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::Result;
use crate::db::Pool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    /// File (or gallery folder) as it was saved.
    pub path: PathBuf,
    pub name: String,
    pub size: Option<u64>,
    /// `http`, `ytdlp` or `gallerydl`.
    pub kind: Option<String>,
    pub package: Option<String>,
    /// Unix milliseconds.
    pub finished_at: i64,
    /// The file or folder is still there.
    pub exists: bool,
}

/// What the LinkGrabber shows for a link downloaded before: the latest time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PreviousDownload {
    pub finished_at: i64,
    pub path: PathBuf,
    pub exists: bool,
}

/// What a finished download leaves in the history.
#[derive(Debug, Clone)]
pub struct Finished<'a> {
    pub url: &'a str,
    pub path: &'a Path,
    pub size: Option<u64>,
    pub kind: &'a str,
    pub package: Option<&'a str>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    url: String,
    final_path: String,
    name: Option<String>,
    size: Option<i64>,
    kind: Option<String>,
    package: Option<String>,
    finished_at: i64,
}

impl Row {
    fn into_entry(self) -> HistoryEntry {
        let path = PathBuf::from(&self.final_path);
        HistoryEntry {
            id: self.id,
            exists: path.exists(),
            name: self.name.unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
            url: self.url,
            path,
            size: self.size.map(|s| s as u64),
            kind: self.kind,
            package: self.package,
            finished_at: self.finished_at,
        }
    }
}

/// The URL as history compares it: parsed and without its `#fragment`, so
/// `page#comments` and `page` are the same download.
fn normalize(url: &str) -> String {
    match Url::parse(url.trim()) {
        Ok(mut u) => {
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => url.trim().to_owned(),
    }
}

fn url_hash(url: &str) -> String {
    Sha256::digest(normalize(url).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub async fn record(pool: &Pool, f: &Finished<'_>) -> Result<()> {
    let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned());
    sqlx::query(
        "INSERT INTO history (url_hash, url, final_path, size, name, kind, package)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(url_hash(f.url))
    .bind(normalize(f.url))
    .bind(f.path.to_string_lossy().into_owned())
    .bind(f.size.map(|s| s as i64))
    .bind(name)
    .bind(f.kind)
    .bind(f.package)
    .execute(pool)
    .await?;
    Ok(())
}

/// Newest first. `search` matches name, URL, folder or package, case-insensitively.
pub async fn list(pool: &Pool, search: Option<&str>, limit: u32) -> Result<Vec<HistoryEntry>> {
    let pattern = search.map(str::trim).filter(|s| !s.is_empty()).map(|s| {
        format!(
            "%{}%",
            s.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    });
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, url, final_path, name, size, kind, package, finished_at FROM history
         WHERE ?1 IS NULL
            OR name LIKE ?1 ESCAPE '\\' OR url LIKE ?1 ESCAPE '\\'
            OR final_path LIKE ?1 ESCAPE '\\' OR package LIKE ?1 ESCAPE '\\'
         ORDER BY finished_at DESC, id DESC
         LIMIT ?2",
    )
    .bind(pattern)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Row::into_entry).collect())
}

pub async fn get(pool: &Pool, id: i64) -> Result<Option<HistoryEntry>> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT id, url, final_path, name, size, kind, package, finished_at FROM history WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Row::into_entry))
}

pub async fn remove(pool: &Pool, ids: &[i64]) -> Result<()> {
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query("DELETE FROM history WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn clear(pool: &Pool) -> Result<()> {
    sqlx::query("DELETE FROM history").execute(pool).await?;
    Ok(())
}

/// The latest finished download for each of `urls` that has one, keyed by
/// the URL as given.
pub async fn previous(pool: &Pool, urls: &[&str]) -> Result<HashMap<String, PreviousDownload>> {
    let mut found = HashMap::new();
    for url in urls {
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT finished_at, final_path FROM history WHERE url_hash = ?
             ORDER BY finished_at DESC, id DESC LIMIT 1",
        )
        .bind(url_hash(url))
        .fetch_optional(pool)
        .await?;
        if let Some((finished_at, path)) = row {
            let path = PathBuf::from(path);
            found.insert(
                (*url).to_owned(),
                PreviousDownload {
                    finished_at,
                    exists: path.exists(),
                    path,
                },
            );
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> (tempfile::TempDir, Pool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::open(&dir.path().join("t.db")).await.unwrap();
        (dir, pool)
    }

    fn finished<'a>(url: &'a str, path: &'a Path) -> Finished<'a> {
        Finished {
            url,
            path,
            size: Some(10),
            kind: "http",
            package: Some("Pkg"),
        }
    }

    #[test]
    fn fragments_do_not_count() {
        assert_eq!(
            url_hash("https://x.test/a#top"),
            url_hash("https://x.test/a")
        );
        assert_ne!(
            url_hash("https://x.test/a?b=1"),
            url_hash("https://x.test/a")
        );
    }

    #[tokio::test]
    async fn records_lists_and_finds_previous_downloads() {
        let (dir, pool) = pool().await;
        let kept = dir.path().join("kept.zip");
        std::fs::write(&kept, "x").unwrap();
        let gone = dir.path().join("gone_100%.zip");
        record(&pool, &finished("https://x.test/kept.zip", &kept))
            .await
            .unwrap();
        record(&pool, &finished("https://x.test/gone.zip#frag", &gone))
            .await
            .unwrap();

        let all = list(&pool, None, 100).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "gone_100%.zip", "newest first");
        assert!(!all[0].exists);
        assert!(all[1].exists);
        assert_eq!(all[1].package.as_deref(), Some("Pkg"));

        // LIKE wildcards in the search are literal.
        assert_eq!(list(&pool, Some("100%"), 100).await.unwrap().len(), 1);
        assert_eq!(list(&pool, Some("_"), 100).await.unwrap().len(), 1);
        assert_eq!(list(&pool, Some("KEPT"), 100).await.unwrap().len(), 1);

        let found = previous(
            &pool,
            &[
                "https://x.test/kept.zip",
                "https://x.test/gone.zip",
                "https://x.test/new",
            ],
        )
        .await
        .unwrap();
        assert!(found["https://x.test/kept.zip"].exists);
        assert!(!found["https://x.test/gone.zip"].exists);
        assert!(!found.contains_key("https://x.test/new"));

        remove(&pool, &[all[0].id]).await.unwrap();
        assert_eq!(list(&pool, None, 100).await.unwrap().len(), 1);
        clear(&pool).await.unwrap();
        assert!(list(&pool, None, 100).await.unwrap().is_empty());
    }
}
