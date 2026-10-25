//! External tools: yt-dlp, gallery-dl and ffmpeg, downloaded on first use
//! into `<data>/tools/` and kept up to date from their releases (GitHub, and
//! Codeberg for gallery-dl).
//!
//! Each install goes into its own folder (`tools/yt-dlp/<release>-<sha>/`) and
//! the `tools` row points at the active one, so an update never touches files
//! a running download is using, and rolling back is a row update. Downloads
//! are verified against the release's published SHA-256 list before they are
//! unpacked, and the new binary must run (`--version`) before it is switched
//! to. The previous install is kept for rollback; older ones are deleted.

mod github;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::db::Pool;
use crate::http::filename;
use crate::{Error, Result, process};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum ToolName {
    Ytdlp,
    Gallerydl,
    Ffmpeg,
}

impl ToolName {
    pub const ALL: [ToolName; 3] = [ToolName::Ytdlp, ToolName::Gallerydl, ToolName::Ffmpeg];

    /// Name in the database and on disk.
    pub fn as_str(self) -> &'static str {
        match self {
            ToolName::Ytdlp => "yt-dlp",
            ToolName::Gallerydl => "gallery-dl",
            ToolName::Ffmpeg => "ffmpeg",
        }
    }

    fn exe(self) -> &'static str {
        match self {
            ToolName::Ytdlp => "yt-dlp.exe",
            ToolName::Gallerydl => "gallery-dl.exe",
            ToolName::Ffmpeg => "ffmpeg.exe",
        }
    }

    fn version_arg(self) -> &'static str {
        match self {
            ToolName::Ytdlp | ToolName::Gallerydl => "--version",
            ToolName::Ffmpeg => "-version",
        }
    }

    /// Extractor tools break when sites change, so background checks install
    /// their updates. ffmpeg is large and rarely matters.
    fn auto_updates(self) -> bool {
        matches!(self, ToolName::Ytdlp | ToolName::Gallerydl)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum ToolChannel {
    Stable,
    Nightly,
}

impl ToolChannel {
    fn as_str(self) -> &'static str {
        match self {
            ToolChannel::Stable => "stable",
            ToolChannel::Nightly => "nightly",
        }
    }
}

/// Release hosts. Codeberg (Forgejo) answers with the same release JSON as
/// GitHub, under its own API root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Forge {
    GitHub,
    Codeberg,
}

/// Where a tool's releases come from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Upstream {
    pub forge: Forge,
    pub repo: &'static str,
    pub asset: &'static str,
    /// Checksum file in the release. `None` uses the SHA-256 `digest` GitHub
    /// computes for every asset, for repos that publish no list.
    pub sums: Option<&'static str>,
    /// The asset is a zip whose `bin/` folder holds the tool.
    pub zipped: bool,
}

fn upstream(name: ToolName, channel: ToolChannel) -> Upstream {
    match (name, channel) {
        (ToolName::Ytdlp, ToolChannel::Stable) => Upstream {
            forge: Forge::GitHub,
            repo: "yt-dlp/yt-dlp",
            asset: "yt-dlp.exe",
            sums: Some("SHA2-256SUMS"),
            zipped: false,
        },
        (ToolName::Ytdlp, ToolChannel::Nightly) => Upstream {
            forge: Forge::GitHub,
            repo: "yt-dlp/yt-dlp-nightly-builds",
            asset: "yt-dlp.exe",
            sums: Some("SHA2-256SUMS"),
            zipped: false,
        },
        // gallery-dl moved to Codeberg; its GitHub releases no longer carry
        // binaries.
        (ToolName::Gallerydl, ToolChannel::Stable) => Upstream {
            forge: Forge::Codeberg,
            repo: "mikf/gallery-dl",
            asset: "gallery-dl.exe",
            sums: Some("SHA256SUMS"),
            zipped: false,
        },
        // Daily builds of the main branch, without a checksum file.
        (ToolName::Gallerydl, ToolChannel::Nightly) => Upstream {
            forge: Forge::GitHub,
            repo: "gdl-org/builds",
            asset: "gallery-dl_windows.exe",
            sums: None,
            zipped: false,
        },
        // The builds yt-dlp recommends. The shared build is half the size of
        // the static one; its DLLs sit next to ffmpeg.exe in `bin/`.
        (ToolName::Ffmpeg, _) => Upstream {
            forge: Forge::GitHub,
            repo: "yt-dlp/FFmpeg-Builds",
            asset: "ffmpeg-master-latest-win64-gpl-shared.zip",
            sums: Some("checksums.sha256"),
            zipped: true,
        },
    }
}

#[derive(Debug, Clone)]
pub struct ToolsConfig {
    /// GitHub API root; tests point this at a local server.
    pub api_base: Url,
    /// Codeberg API root (`…/api/v1/`).
    pub codeberg_base: Url,
    /// Background checks run at most this often per tool.
    pub check_interval: Duration,
    /// Install yt-dlp and gallery-dl updates found by background checks.
    /// ffmpeg is only ever updated on request: it is large and rarely matters.
    pub auto_update: bool,
    /// Run the new binary before switching to it. Tests serve fake binaries
    /// and turn this off; the version is then the release name.
    pub verify_runs: bool,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            api_base: Url::parse("https://api.github.com/").expect("valid URL"),
            codeberg_base: Url::parse("https://codeberg.org/api/v1/").expect("valid URL"),
            check_interval: Duration::from_secs(24 * 60 * 60),
            auto_update: true,
            verify_runs: true,
        }
    }
}

/// What a tool is busy with right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolTask {
    Checking,
    Downloading { downloaded: u64, total: Option<u64> },
    Installing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: ToolName,
    pub channel: ToolChannel,
    /// Path of the active executable, when installed.
    pub path: Option<PathBuf>,
    /// What the binary reports (`yt-dlp --version`).
    pub version: Option<String>,
    /// The upstream release it was installed from.
    pub release: Option<String>,
    pub installed_at: Option<i64>,
    /// Newest release seen at the last check.
    pub latest_release: Option<String>,
    pub update_available: bool,
    pub last_check: Option<i64>,
    /// Version a rollback would return to.
    pub previous_version: Option<String>,
    pub task: Option<ToolTask>,
    /// Why the last check or install failed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolEvent {
    Changed { tool: ToolInfo },
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
struct ToolRow {
    channel: String,
    path: Option<String>,
    version: Option<String>,
    release: Option<String>,
    sha256: Option<String>,
    installed_at: Option<i64>,
    last_check: Option<i64>,
    latest_release: Option<String>,
    latest_sha256: Option<String>,
    previous_path: Option<String>,
    previous_version: Option<String>,
}

impl ToolRow {
    fn channel(&self) -> ToolChannel {
        match &*self.channel {
            "nightly" => ToolChannel::Nightly,
            _ => ToolChannel::Stable,
        }
    }
}

#[derive(Default)]
struct Live {
    task: Option<ToolTask>,
    error: Option<String>,
}

/// Cheap to clone; all clones share state.
#[derive(Clone)]
pub struct Tools {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Tools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tools")
            .field("dir", &self.inner.dir)
            .finish()
    }
}

struct Inner {
    pool: Pool,
    client: reqwest::Client,
    dir: PathBuf,
    config: ToolsConfig,
    events: broadcast::Sender<ToolEvent>,
    /// One install or check per tool at a time.
    locks: HashMap<ToolName, tokio::sync::Mutex<()>>,
    live: Mutex<HashMap<ToolName, Live>>,
}

impl Tools {
    pub async fn start(pool: Pool, dir: PathBuf, config: ToolsConfig) -> Result<Self> {
        for name in ToolName::ALL {
            sqlx::query("INSERT OR IGNORE INTO tools (name) VALUES (?)")
                .bind(name.as_str())
                .execute(&pool)
                .await?;
        }
        let (events, _) = broadcast::channel(64);
        let tools = Self {
            inner: Arc::new(Inner {
                client: crate::http::client(None)?,
                pool,
                dir,
                config,
                events,
                locks: ToolName::ALL
                    .into_iter()
                    .map(|n| (n, tokio::sync::Mutex::new(())))
                    .collect(),
                live: Mutex::new(HashMap::new()),
            }),
        };
        // Someone may have deleted the tools folder; forget installs that are gone.
        for name in ToolName::ALL {
            let row = tools.row(name).await?;
            if row.path.as_deref().is_some_and(|p| !Path::new(p).is_file()) {
                tracing::warn!(
                    tool = name.as_str(),
                    "installed tool is missing; forgetting it"
                );
                sqlx::query(
                    "UPDATE tools SET path = previous_path, version = previous_version,
                         release = previous_release, sha256 = previous_sha256,
                         previous_path = NULL, previous_version = NULL,
                         previous_release = NULL, previous_sha256 = NULL
                     WHERE name = ?",
                )
                .bind(name.as_str())
                .execute(&tools.inner.pool)
                .await?;
                let row = tools.row(name).await?;
                if row.path.as_deref().is_some_and(|p| !Path::new(p).is_file()) {
                    tools.forget(name).await?;
                }
            }
        }
        Ok(tools)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ToolEvent> {
        self.inner.events.subscribe()
    }

    pub async fn list(&self) -> Result<Vec<ToolInfo>> {
        let mut out = Vec::new();
        for name in ToolName::ALL {
            out.push(self.info(name).await?);
        }
        Ok(out)
    }

    pub async fn info(&self, name: ToolName) -> Result<ToolInfo> {
        let row = self.row(name).await?;
        let live = self.live();
        let live = live.get(&name);
        Ok(ToolInfo {
            name,
            channel: row.channel(),
            update_available: row.path.is_some()
                && row.latest_sha256.is_some()
                && row.latest_sha256 != row.sha256,
            path: row.path.map(PathBuf::from),
            version: row.version,
            release: row.release,
            installed_at: row.installed_at,
            latest_release: row.latest_release,
            last_check: row.last_check,
            previous_version: row.previous_path.and(row.previous_version),
            task: live.and_then(|l| l.task.clone()),
            error: live.and_then(|l| l.error.clone()),
        })
    }

    /// The active executable, if installed.
    pub async fn path(&self, name: ToolName) -> Result<Option<PathBuf>> {
        Ok(self
            .row(name)
            .await?
            .path
            .map(PathBuf::from)
            .filter(|p| p.is_file()))
    }

    /// The executable, installing it first if needed.
    pub async fn ensure(&self, name: ToolName) -> Result<PathBuf> {
        if let Some(path) = self.path(name).await? {
            return Ok(path);
        }
        self.install(name).await?;
        self.path(name)
            .await?
            .ok_or_else(|| Error::Tool(format!("{} could not be installed", name.as_str())))
    }

    /// Install the newest release, or update to it. Does nothing when the
    /// newest release is already active.
    pub async fn install(&self, name: ToolName) -> Result<ToolInfo> {
        let _guard = self.inner.locks[&name].lock().await;
        let result = self.install_locked(name).await;
        self.finish_task(name, result.as_ref().err()).await;
        result?;
        self.info(name).await
    }

    /// Look up the newest release without installing it.
    pub async fn check(&self, name: ToolName) -> Result<ToolInfo> {
        let _guard = self.inner.locks[&name].lock().await;
        self.set_task(name, Some(ToolTask::Checking)).await;
        let channel = self.row(name).await?.channel();
        let result = self.fetch_latest(name, channel).await;
        self.finish_task(name, result.as_ref().err()).await;
        result?;
        self.info(name).await
    }

    /// Switch back to the previous install, keeping the current one as the
    /// new "previous" so the rollback can itself be undone.
    pub async fn rollback(&self, name: ToolName) -> Result<ToolInfo> {
        let _guard = self.inner.locks[&name].lock().await;
        let row = self.row(name).await?;
        if !row
            .previous_path
            .as_deref()
            .is_some_and(|p| Path::new(p).is_file())
        {
            return Err(Error::Tool(format!(
                "there is no earlier {} to roll back to",
                name.as_str()
            )));
        }
        sqlx::query(
            "UPDATE tools SET
                 path = previous_path, version = previous_version,
                 release = previous_release, sha256 = previous_sha256,
                 previous_path = path, previous_version = version,
                 previous_release = release, previous_sha256 = sha256,
                 installed_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
             WHERE name = ?",
        )
        .bind(name.as_str())
        .execute(&self.inner.pool)
        .await?;
        tracing::info!(tool = name.as_str(), to = ?row.previous_version, "rolled back");
        self.changed(name).await;
        self.info(name).await
    }

    /// Pick the release channel; the next check or install uses it.
    pub async fn set_channel(&self, name: ToolName, channel: ToolChannel) -> Result<ToolInfo> {
        sqlx::query(
            "UPDATE tools SET channel = ?, latest_release = NULL, latest_sha256 = NULL, last_check = NULL
             WHERE name = ?",
        )
        .bind(channel.as_str())
        .bind(name.as_str())
        .execute(&self.inner.pool)
        .await?;
        self.changed(name).await;
        self.info(name).await
    }

    /// Check installed tools that are due, and apply yt-dlp and gallery-dl
    /// updates if enabled. Errors are recorded on the tool, not returned.
    pub async fn check_due(&self) {
        for name in ToolName::ALL {
            let Ok(row) = self.row(name).await else {
                continue;
            };
            if row.path.is_none() {
                continue;
            }
            let interval = self.inner.config.check_interval.as_millis() as i64;
            if row.last_check.is_some_and(|at| now_ms() - at < interval) {
                continue;
            }
            match self.check(name).await {
                Ok(info) if info.update_available => {
                    if name.auto_updates() && self.inner.config.auto_update {
                        tracing::info!(tool = name.as_str(), to = ?info.latest_release, "updating");
                        if let Err(e) = self.install(name).await {
                            tracing::warn!(tool = name.as_str(), error = %e, "update failed");
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(tool = name.as_str(), error = %e, "update check failed"),
            }
        }
    }

    /// Run [`Self::check_due`] shortly after start and then every hour.
    pub fn spawn_background_checks(&self, shutdown: CancellationToken) {
        let tools = self.clone();
        tokio::spawn(async move {
            let mut delay = Duration::from_secs(10);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                tools.check_due().await;
                delay = Duration::from_secs(60 * 60);
            }
        });
    }

    async fn install_locked(&self, name: ToolName) -> Result<()> {
        self.set_task(name, Some(ToolTask::Checking)).await;
        let row = self.row(name).await?;
        let latest = self.fetch_latest(name, row.channel()).await?;
        let current = row.path.as_deref().is_some_and(|p| Path::new(p).is_file());
        if current && row.sha256.as_deref() == Some(&*latest.sha256) {
            return Ok(());
        }
        let up = upstream(name, row.channel());
        let tool_dir = self.inner.dir.join(name.as_str());
        let short = &latest.sha256[..8];
        let target = tool_dir.join(filename::sanitize(&format!("{}-{short}", latest.release)));
        let exe = target.join(name.exe());

        // A folder for this exact release is left from an earlier install
        // (say, before a rollback); it was verified then.
        if !exe.is_file() {
            let staging = self.inner.dir.join(".staging");
            tokio::fs::create_dir_all(&staging).await?;
            let download = staging.join(format!("{}-{short}.download", name.as_str()));
            self.set_task(
                name,
                Some(ToolTask::Downloading {
                    downloaded: 0,
                    total: latest.size,
                }),
            )
            .await;
            let fetched = self.download_verified(name, &latest, &download).await;
            if let Err(e) = fetched {
                let _ = tokio::fs::remove_file(&download).await;
                return Err(e);
            }

            self.set_task(name, Some(ToolTask::Installing)).await;
            let unpack = staging.join(format!("{}-{short}", name.as_str()));
            let _ = tokio::fs::remove_dir_all(&unpack).await;
            tokio::fs::create_dir_all(&unpack).await?;
            let placed = if up.zipped {
                let (zip, dest) = (download.clone(), unpack.clone());
                tokio::task::spawn_blocking(move || extract_bin(&zip, &dest))
                    .await
                    .map_err(|e| Error::Tool(e.to_string()))?
            } else {
                tokio::fs::rename(&download, unpack.join(name.exe()))
                    .await
                    .map_err(Error::from)
            };
            let _ = tokio::fs::remove_file(&download).await;
            if let Err(e) = placed {
                let _ = tokio::fs::remove_dir_all(&unpack).await;
                return Err(e);
            }
            if !unpack.join(name.exe()).is_file() {
                let _ = tokio::fs::remove_dir_all(&unpack).await;
                return Err(Error::Tool(format!("{} has no {}", up.asset, name.exe())));
            }
            tokio::fs::create_dir_all(&tool_dir).await?;
            let _ = tokio::fs::remove_dir_all(&target).await;
            tokio::fs::rename(&unpack, &target).await?;
        }

        self.set_task(name, Some(ToolTask::Installing)).await;
        let version = if self.inner.config.verify_runs {
            match detect_version(name, &exe).await {
                Ok(v) => v,
                Err(e) => {
                    let _ = tokio::fs::remove_dir_all(&target).await;
                    return Err(Error::Tool(format!(
                        "the downloaded {} does not run: {e}",
                        name.as_str()
                    )));
                }
            }
        } else {
            latest.release.clone()
        };

        let exe_str = exe.to_string_lossy().into_owned();
        let keep_previous = row.path.is_some() && row.path.as_deref() != Some(&*exe_str);
        sqlx::query(
            "UPDATE tools SET
                 previous_path    = CASE WHEN ?1 THEN path ELSE previous_path END,
                 previous_version = CASE WHEN ?1 THEN version ELSE previous_version END,
                 previous_release = CASE WHEN ?1 THEN release ELSE previous_release END,
                 previous_sha256  = CASE WHEN ?1 THEN sha256 ELSE previous_sha256 END,
                 path = ?2, version = ?3, release = ?4, sha256 = ?5,
                 installed_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
             WHERE name = ?6",
        )
        .bind(keep_previous)
        .bind(&exe_str)
        .bind(&version)
        .bind(&latest.release)
        .bind(&latest.sha256)
        .bind(name.as_str())
        .execute(&self.inner.pool)
        .await?;
        tracing::info!(tool = name.as_str(), %version, release = %latest.release, "tool installed");
        self.prune(name).await;
        Ok(())
    }

    async fn fetch_latest(&self, name: ToolName, channel: ToolChannel) -> Result<github::Latest> {
        let up = upstream(name, channel);
        let api = match up.forge {
            Forge::GitHub => &self.inner.config.api_base,
            Forge::Codeberg => &self.inner.config.codeberg_base,
        };
        let latest = github::latest(&self.inner.client, api, &up).await?;
        sqlx::query(
            "UPDATE tools SET latest_release = ?, latest_sha256 = ?,
                 last_check = CAST(unixepoch('subsec') * 1000 AS INTEGER)
             WHERE name = ?",
        )
        .bind(&latest.release)
        .bind(&latest.sha256)
        .bind(name.as_str())
        .execute(&self.inner.pool)
        .await?;
        Ok(latest)
    }

    async fn download_verified(
        &self,
        name: ToolName,
        latest: &github::Latest,
        to: &Path,
    ) -> Result<()> {
        let response = self.inner.client.get(latest.url.clone()).send().await?;
        crate::http::check_status(response.status(), &latest.url)?;
        let total = response.content_length().or(latest.size);
        let mut file = tokio::fs::File::create(to).await?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0u64;
        let mut last_report = Instant::now();
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            if last_report.elapsed() >= Duration::from_millis(200) {
                last_report = Instant::now();
                self.set_task(name, Some(ToolTask::Downloading { downloaded, total }))
                    .await;
            }
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);

        let actual = hex(&hasher.finalize());
        if actual != latest.sha256 {
            return Err(Error::ChecksumMismatch {
                file: latest
                    .url
                    .path()
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_owned(),
                expected: latest.sha256.clone(),
                actual,
            });
        }
        Ok(())
    }

    /// Delete install folders other than the active and previous ones. Folders
    /// still in use by a running download fail to delete and are retried next time.
    async fn prune(&self, name: ToolName) {
        let Ok(row) = self.row(name).await else {
            return;
        };
        let keep: Vec<PathBuf> = [row.path, row.previous_path]
            .into_iter()
            .flatten()
            .filter_map(|p| Path::new(&p).parent().map(Path::to_owned))
            .collect();
        let Ok(mut entries) = tokio::fs::read_dir(self.inner.dir.join(name.as_str())).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir()
                && !keep.contains(&path)
                && let Err(e) = tokio::fs::remove_dir_all(&path).await
            {
                tracing::debug!(path = %path.display(), error = %e, "could not prune old tool");
            }
        }
    }

    async fn forget(&self, name: ToolName) -> Result<()> {
        sqlx::query(
            "UPDATE tools SET path = NULL, version = NULL, release = NULL, sha256 = NULL, installed_at = NULL
             WHERE name = ?",
        )
        .bind(name.as_str())
        .execute(&self.inner.pool)
        .await?;
        Ok(())
    }

    async fn row(&self, name: ToolName) -> Result<ToolRow> {
        Ok(sqlx::query_as("SELECT * FROM tools WHERE name = ?")
            .bind(name.as_str())
            .fetch_optional(&self.inner.pool)
            .await?
            .unwrap_or_default())
    }

    fn live(&self) -> MutexGuard<'_, HashMap<ToolName, Live>> {
        self.inner.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn set_task(&self, name: ToolName, task: Option<ToolTask>) {
        {
            let mut live = self.live();
            let entry = live.entry(name).or_default();
            if task.is_some() {
                entry.error = None;
            }
            entry.task = task;
        }
        self.changed(name).await;
    }

    async fn finish_task(&self, name: ToolName, error: Option<&Error>) {
        if let Some(e) = error {
            tracing::warn!(tool = name.as_str(), error = %e, "tool task failed");
        }
        {
            let mut live = self.live();
            let entry = live.entry(name).or_default();
            entry.task = None;
            entry.error = error.map(|e| e.to_string());
        }
        self.changed(name).await;
    }

    async fn changed(&self, name: ToolName) {
        if let Ok(tool) = self.info(name).await {
            let _ = self.inner.events.send(ToolEvent::Changed { tool });
        }
    }
}

/// Ask the binary for its version.
async fn detect_version(name: ToolName, exe: &Path) -> Result<String> {
    let mut cmd = process::command(exe);
    cmd.arg(name.version_arg());
    // The first launch of a new PyInstaller exe can sit in an antivirus scan
    // for a minute or more before it even starts.
    let out = process::output(cmd, Duration::from_secs(300), &CancellationToken::new()).await?;
    if !out.status.success() {
        return Err(Error::Tool(out.error_message()));
    }
    parse_version(name, &out.stdout)
        .ok_or_else(|| Error::Tool(format!("unexpected version output: {}", out.stdout.trim())))
}

fn parse_version(name: ToolName, stdout: &str) -> Option<String> {
    let first = stdout.lines().map(str::trim).find(|l| !l.is_empty())?;
    match name {
        ToolName::Ytdlp | ToolName::Gallerydl => Some(first.to_owned()),
        // "ffmpeg version N-121490-g0123abcd-20261024 Copyright (c) ..."
        ToolName::Ffmpeg => first
            .strip_prefix("ffmpeg version ")
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_owned),
    }
}

/// Unpack every file under a `bin/` folder of the zip, flattened into `dest`.
fn extract_bin(zip: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| Error::Tool(format!("bad archive: {e}")))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::Tool(format!("bad archive: {e}")))?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let in_bin = path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|dir| dir == "bin");
        if !entry.is_file() || !in_bin {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let mut out = std::fs::File::create(dest.join(file_name))?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(
            parse_version(ToolName::Ytdlp, "2026.10.21\n").as_deref(),
            Some("2026.10.21")
        );
        let ffmpeg =
            "ffmpeg version N-121490-g0123abcd-20261024 Copyright (c) 2000-2026\nbuilt with gcc";
        assert_eq!(
            parse_version(ToolName::Ffmpeg, ffmpeg).as_deref(),
            Some("N-121490-g0123abcd-20261024")
        );
        assert_eq!(parse_version(ToolName::Ffmpeg, "garbage"), None);
    }

    #[test]
    fn hex_encodes() {
        assert_eq!(hex(&[0x00, 0xab, 0x10]), "00ab10");
    }
}
