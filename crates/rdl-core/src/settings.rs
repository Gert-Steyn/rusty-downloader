//! Typed key/value settings stored as JSON in the `settings` table.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Result;
use crate::db::Pool;

pub const CLIPBOARD_WATCH: &str = "linkgrabber.clipboard_watch";

pub async fn get<T: DeserializeOwned>(pool: &Pool, key: &str) -> Result<Option<T>> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(raw.map(|json| serde_json::from_str(&json)).transpose()?)
}

pub async fn set<T: Serialize>(pool: &Pool, key: &str, value: &T) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value_json) VALUES (?, ?)
         ON CONFLICT (key) DO UPDATE SET value_json = excluded.value_json",
    )
    .bind(key)
    .bind(serde_json::to_string(value)?)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::open(&dir.path().join("t.db")).await.unwrap();

        assert_eq!(get::<bool>(&pool, CLIPBOARD_WATCH).await.unwrap(), None);
        set(&pool, CLIPBOARD_WATCH, &true).await.unwrap();
        set(&pool, CLIPBOARD_WATCH, &false).await.unwrap();
        assert_eq!(
            get::<bool>(&pool, CLIPBOARD_WATCH).await.unwrap(),
            Some(false)
        );
    }
}
