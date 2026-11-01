use std::path::PathBuf;
use std::sync::{RwLock, RwLockReadGuard};

use rdl_core::http::SegmentConfig;
use rdl_core::linkgrabber::LinkGrabber;
use rdl_core::logging::{self, WorkerGuard};
use rdl_core::manager::{DEFAULT_QUEUE, Manager as DownloadManager, ManagerConfig};
use rdl_core::network::Network;
use rdl_core::secrets::{CredentialManager, MemorySecrets, Secrets};
use rdl_core::settings::Settings;
use rdl_core::tools::{Tools, ToolsConfig};
use rdl_core::{db, paths::AppPaths};
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

/// Long-lived services shared by all commands.
pub struct AppState {
    pub paths: AppPaths,
    pub db: db::Pool,
    pub downloads: DownloadManager,
    pub grabber: LinkGrabber,
    pub tools: Tools,
    /// Proxies and the rules picking them, shared by everything that connects.
    pub network: Network,
    /// Stops background work (tool update checks) on exit.
    pub shutdown: CancellationToken,
    /// The saved preferences, as last applied.
    settings: RwLock<Settings>,
    _log_guard: WorkerGuard,
}

impl AppState {
    pub async fn init() -> rdl_core::Result<Self> {
        let paths = AppPaths::resolve()?;
        paths.ensure_dirs()?;
        let log_guard = logging::init(&paths.logs_dir)?;

        tracing::info!(
            version = rdl_core::VERSION,
            portable = paths.portable,
            data_dir = %paths.data_dir.display(),
            "starting rusty-downloader"
        );

        let db = db::open(&paths.db_file).await?;
        tracing::info!(schema = db::schema_version(&db).await?, "database ready");
        let settings = Settings::load(&db).await?;
        let network = Network::load(db.clone(), secret_store(), None).await?;

        let tools = Tools::start(
            db.clone(),
            paths.tools_dir.clone(),
            ToolsConfig {
                network: Some(network.clone()),
                ..ToolsConfig::default()
            },
        )
        .await?;
        let shutdown = CancellationToken::new();
        tools.spawn_background_checks(shutdown.clone());
        let downloads = DownloadManager::start(
            db.clone(),
            ManagerConfig {
                tools: Some(tools.clone()),
                archives_dir: Some(paths.archives_dir.clone()),
                network: Some(network.clone()),
                segment: SegmentConfig {
                    max_connections: settings.max_connections as usize,
                    ..SegmentConfig::default()
                },
                ..ManagerConfig::default()
            },
        )
        .await?;
        let grabber =
            LinkGrabber::start(db.clone(), Some(network.clone()), Some(tools.clone())).await?;

        let state = Self {
            paths,
            db,
            downloads,
            grabber,
            tools,
            network,
            shutdown,
            settings: RwLock::new(settings.clone()),
            _log_guard: log_guard,
        };
        state.apply(&settings).await?;
        Ok(state)
    }

    pub fn settings(&self) -> RwLockReadGuard<'_, Settings> {
        self.settings.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Check, save and apply new preferences; returns them as stored.
    pub async fn update_settings(&self, new: Settings) -> rdl_core::Result<Settings> {
        let new = new.validated()?;
        new.save(&self.db).await?;
        self.apply(&new).await?;
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = new.clone();
        tracing::info!(?new, "settings saved");
        Ok(new)
    }

    /// Push the preferences the core services hold into them.
    async fn apply(&self, settings: &Settings) -> rdl_core::Result<()> {
        self.downloads
            .set_queue_limit(DEFAULT_QUEUE, settings.max_active)
            .await?;
        self.downloads
            .set_max_connections(settings.max_connections as usize);
        self.grabber.set_warn_duplicates(settings.warn_duplicates);
        self.downloads
            .set_speed_limit(settings.active_speed_limit_now().limit);
        Ok(())
    }

    pub fn clipboard_watch(&self) -> bool {
        self.settings().clipboard_watch
    }

    pub async fn set_clipboard_watch(&self, enabled: bool) -> rdl_core::Result<()> {
        let mut settings = self.settings().clone();
        settings.clipboard_watch = enabled;
        self.update_settings(settings).await?;
        Ok(())
    }

    /// Folder for new packages: the chosen one, else the user's Downloads.
    pub fn download_dir(&self, app: &AppHandle) -> Result<PathBuf, String> {
        if let Some(dir) = &self.settings().download_dir {
            return Ok(dir.clone());
        }
        app.path().download_dir().map_err(|e| e.to_string())
    }
}

/// Windows Credential Manager for proxy passwords; if it can't be opened,
/// passwords last only until the app closes (and saved ones can't be read).
fn secret_store() -> std::sync::Arc<dyn Secrets> {
    match CredentialManager::new() {
        Ok(store) => store,
        Err(e) => {
            tracing::error!(error = %e, "Credential Manager unavailable; passwords won't be kept");
            MemorySecrets::new()
        }
    }
}
