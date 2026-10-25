//! Settings: typed key/value pairs stored as JSON in the `settings` table, and
//! the app's preferences as one [`Settings`] value under [`APP`].

use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::db::Pool;
use crate::ratelimit::MIN_RATE;
use crate::speed::{self, ActiveLimit, SpeedRule};
use crate::{Error, Result};

/// Key of the [`Settings`] value.
pub const APP: &str = "app";

/// Where the clipboard switch lived before [`Settings`]; read once as a fallback.
pub const CLIPBOARD_WATCH: &str = "linkgrabber.clipboard_watch";

pub const MAX_ACTIVE: u32 = 20;
pub const MAX_CONNECTIONS: u32 = 32;

/// The user's preferences. Missing fields (older saves) get their defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Folder for new packages; `None` is the user's Downloads folder.
    pub download_dir: Option<PathBuf>,
    /// Downloads running at once (in the main queue).
    pub max_active: u32,
    /// Connections per plain download.
    pub max_connections: u32,
    /// Add links copied anywhere to the LinkGrabber.
    pub clipboard_watch: bool,
    /// Offer copied links with an "Add" button instead of adding them.
    pub clipboard_ask: bool,
    /// Copied links matching one of these (`*` matches anything) are ignored.
    pub clipboard_ignore: Vec<String>,
    pub start_with_windows: bool,
    /// When started with Windows, stay in the tray.
    pub start_minimized: bool,
    /// Closing the window hides it to the tray; Quit is in the tray menu.
    pub close_to_tray: bool,
    /// Windows notification when a package finishes.
    pub notify_finished: bool,
    /// Windows notification when a download fails.
    pub notify_failed: bool,
    /// Flag links in the LinkGrabber that were downloaded before, and untick
    /// them while the file is still there.
    pub warn_duplicates: bool,
    /// Don't let Windows go to sleep while downloads are running.
    pub keep_awake: bool,
    /// The status bar's switch: limit the speed to `speed_limit` now.
    pub speed_limit_on: bool,
    /// Bytes per second while the switch is on.
    pub speed_limit: u64,
    /// Limits that apply by themselves at set times, while the switch is off.
    pub speed_schedule: Vec<SpeedRule>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: None,
            max_active: 3,
            max_connections: 8,
            clipboard_watch: true,
            clipboard_ask: false,
            clipboard_ignore: Vec::new(),
            start_with_windows: false,
            start_minimized: true,
            close_to_tray: true,
            notify_finished: true,
            notify_failed: true,
            warn_duplicates: true,
            keep_awake: true,
            speed_limit_on: false,
            speed_limit: 2 * 1024 * 1024,
            speed_schedule: Vec::new(),
        }
    }
}

impl Settings {
    pub async fn load(pool: &Pool) -> Result<Self> {
        if let Some(settings) = get::<Settings>(pool, APP).await? {
            return Ok(settings);
        }
        Ok(Self {
            clipboard_watch: get(pool, CLIPBOARD_WATCH).await?.unwrap_or(true),
            ..Self::default()
        })
    }

    /// Check and tidy values from the UI (trimmed, blank patterns dropped).
    pub fn validated(mut self) -> Result<Self> {
        let invalid = |m: String| Err(Error::InvalidInput(m));
        if !(1..=MAX_ACTIVE).contains(&self.max_active) {
            return invalid(format!(
                "Downloads at once must be between 1 and {MAX_ACTIVE}."
            ));
        }
        if !(1..=MAX_CONNECTIONS).contains(&self.max_connections) {
            return invalid(format!(
                "Connections per download must be between 1 and {MAX_CONNECTIONS}."
            ));
        }
        if let Some(dir) = &self.download_dir
            && !dir.is_absolute()
        {
            return invalid(format!("\"{}\" is not a full folder path.", dir.display()));
        }
        if self.speed_limit < MIN_RATE {
            return invalid(format!(
                "The speed limit must be at least {} KB/s.",
                MIN_RATE / 1024
            ));
        }
        if self.speed_schedule.len() > speed::MAX_RULES {
            return invalid(format!("At most {} scheduled limits.", speed::MAX_RULES));
        }
        self.speed_schedule = self
            .speed_schedule
            .into_iter()
            .map(SpeedRule::validated)
            .collect::<Result<_>>()?;
        self.clipboard_ignore = self
            .clipboard_ignore
            .iter()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect();
        Ok(self)
    }

    pub async fn save(&self, pool: &Pool) -> Result<()> {
        set(pool, APP, self).await
    }

    /// The speed limit that applies at `now` (local time).
    pub fn active_speed_limit(&self, now: chrono::NaiveDateTime) -> ActiveLimit {
        speed::active_limit(
            self.speed_limit_on,
            self.speed_limit,
            &self.speed_schedule,
            now,
        )
    }

    /// The speed limit that applies now.
    pub fn active_speed_limit_now(&self) -> ActiveLimit {
        self.active_speed_limit(chrono::Local::now().naive_local())
    }

    /// Whether a copied link should be left alone.
    pub fn clipboard_ignores(&self, url: &str) -> bool {
        self.clipboard_ignore.iter().any(|p| wildcard_match(p, url))
    }
}

/// Case-insensitive match of the whole `text` against `pattern`, where `*`
/// stands for any run of characters. A pattern without `*` matches anywhere
/// in the text, so `example.com` ignores every link that mentions it.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let text = text.to_lowercase();
    if !pattern.contains('*') {
        return text.contains(&pattern);
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || !text[first.len()..].ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

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

    async fn pool() -> (tempfile::TempDir, Pool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::open(&dir.path().join("t.db")).await.unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn round_trips_and_overwrites() {
        let (_dir, pool) = pool().await;
        assert_eq!(get::<bool>(&pool, CLIPBOARD_WATCH).await.unwrap(), None);
        set(&pool, CLIPBOARD_WATCH, &true).await.unwrap();
        set(&pool, CLIPBOARD_WATCH, &false).await.unwrap();
        assert_eq!(
            get::<bool>(&pool, CLIPBOARD_WATCH).await.unwrap(),
            Some(false)
        );
    }

    #[tokio::test]
    async fn loads_defaults_then_the_old_clipboard_switch_then_saved_settings() {
        let (_dir, pool) = pool().await;
        assert_eq!(Settings::load(&pool).await.unwrap(), Settings::default());

        set(&pool, CLIPBOARD_WATCH, &false).await.unwrap();
        assert!(!Settings::load(&pool).await.unwrap().clipboard_watch);

        let saved = Settings {
            max_active: 5,
            clipboard_watch: true,
            ..Settings::default()
        };
        saved.save(&pool).await.unwrap();
        assert_eq!(Settings::load(&pool).await.unwrap(), saved);
    }

    #[test]
    fn older_saves_get_new_fields_as_defaults() {
        let s: Settings = serde_json::from_str(r#"{"maxActive": 2}"#).unwrap();
        assert_eq!(s.max_active, 2);
        assert!(s.close_to_tray);
        assert!(s.keep_awake, "saves from before 3a keep the computer awake");
    }

    #[test]
    fn validation_bounds_and_tidies() {
        let ok = Settings {
            clipboard_ignore: vec![" *.example.com/* ".into(), "  ".into()],
            ..Settings::default()
        }
        .validated()
        .unwrap();
        assert_eq!(ok.clipboard_ignore, ["*.example.com/*"]);
        for bad in [
            Settings {
                max_active: 0,
                ..Settings::default()
            },
            Settings {
                max_connections: 33,
                ..Settings::default()
            },
            Settings {
                download_dir: Some("C:dl".into()),
                ..Settings::default()
            },
            Settings {
                speed_limit: 10,
                ..Settings::default()
            },
            Settings {
                speed_schedule: vec![SpeedRule {
                    days: vec![],
                    from: "08:00".into(),
                    to: "18:00".into(),
                    limit: 4096,
                }],
                ..Settings::default()
            },
        ] {
            assert!(bad.validated().is_err());
        }
    }

    #[test]
    fn wildcards_match_whole_links() {
        assert!(wildcard_match(
            "*youtube.com/shorts/*",
            "https://www.YouTube.com/shorts/abc"
        ));
        assert!(!wildcard_match(
            "*youtube.com/shorts/*",
            "https://www.youtube.com/watch?v=1"
        ));
        assert!(wildcard_match(
            "https://*.example.com/*.zip",
            "https://cdn.example.com/a/b.zip"
        ));
        assert!(!wildcard_match(
            "https://*.example.com/*.zip",
            "https://cdn.example.com/a/b.zip?x"
        ));
        assert!(wildcard_match("example.com", "https://example.com/x"));
        assert!(wildcard_match("*", "anything"));
        assert!(!wildcard_match("a*a", "a"));
    }
}
