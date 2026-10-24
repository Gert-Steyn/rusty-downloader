//! SQLite storage. Migrations in `crates/rdl-core/migrations` are embedded at
//! compile time and applied on every open.

use std::path::Path;
use std::time::Duration;

use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::Result;

pub type Pool = sqlx::SqlitePool;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Open (creating if needed) the database at `path` and bring its schema up to date.
pub async fn open(path: &Path) -> Result<Pool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;

    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

/// Version of the newest applied migration.
pub async fn schema_version(pool: &Pool) -> Result<i64> {
    let version: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(pool)
            .await?;
    Ok(version.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_db() -> (tempfile::TempDir, Pool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = open(&dir.path().join("test.db")).await.unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn migrations_apply_and_report_latest_version() {
        let (_dir, pool) = temp_db().await;

        let latest = MIGRATOR.iter().map(|m| m.version).max().unwrap();
        assert_eq!(schema_version(&pool).await.unwrap(), latest);
    }

    #[tokio::test]
    async fn reopening_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");

        open(&path).await.unwrap().close().await;
        let pool = open(&path).await.unwrap();

        assert!(schema_version(&pool).await.unwrap() > 0);
    }

    #[tokio::test]
    async fn default_queue_is_seeded() {
        let (_dir, pool) = temp_db().await;

        let name: String = sqlx::query_scalar("SELECT name FROM queues WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(name, "Main");
    }

    #[tokio::test]
    async fn deleting_a_package_cascades_to_downloads_and_segments() {
        let (_dir, pool) = temp_db().await;

        sqlx::query(
            "INSERT INTO packages (id, name, dest_dir, queue_id) VALUES (1, 'p', 'C:\\d', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO downloads (id, package_id, kind, source_url) VALUES (1, 1, 'http', 'https://example.com/f')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO segments (download_id, idx, start_byte, end_byte) VALUES (1, 0, 0, 99)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM packages WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();

        let segments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM segments")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(segments, 0);
    }

    #[tokio::test]
    async fn invalid_download_kind_is_rejected() {
        let (_dir, pool) = temp_db().await;
        sqlx::query(
            "INSERT INTO packages (id, name, dest_dir, queue_id) VALUES (1, 'p', 'C:\\d', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = sqlx::query(
            "INSERT INTO downloads (package_id, kind, source_url) VALUES (1, 'gopher', 'gopher://x')",
        )
        .execute(&pool)
        .await;

        assert!(result.is_err());
    }
}
