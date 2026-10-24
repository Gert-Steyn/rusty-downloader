use std::sync::atomic::{AtomicBool, Ordering};

use rdl_core::linkgrabber::LinkGrabber;
use rdl_core::logging::{self, WorkerGuard};
use rdl_core::manager::{Manager as DownloadManager, ManagerConfig};
use rdl_core::{db, paths::AppPaths, settings};

/// Long-lived services shared by all commands.
pub struct AppState {
    pub paths: AppPaths,
    pub db: db::Pool,
    pub downloads: DownloadManager,
    pub grabber: LinkGrabber,
    /// Mirrors the `linkgrabber.clipboard_watch` setting for the watcher loop.
    clipboard_watch: AtomicBool,
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

        let downloads = DownloadManager::start(db.clone(), ManagerConfig::default()).await?;
        let grabber = LinkGrabber::start(db.clone(), None).await?;
        let clipboard_watch = settings::get(&db, settings::CLIPBOARD_WATCH)
            .await?
            .unwrap_or(true);

        Ok(Self {
            paths,
            db,
            downloads,
            grabber,
            clipboard_watch: AtomicBool::new(clipboard_watch),
            _log_guard: log_guard,
        })
    }

    pub fn clipboard_watch(&self) -> bool {
        self.clipboard_watch.load(Ordering::Relaxed)
    }

    pub async fn set_clipboard_watch(&self, enabled: bool) -> rdl_core::Result<()> {
        settings::set(&self.db, settings::CLIPBOARD_WATCH, &enabled).await?;
        self.clipboard_watch.store(enabled, Ordering::Relaxed);
        tracing::info!(enabled, "clipboard watching changed");
        Ok(())
    }
}
