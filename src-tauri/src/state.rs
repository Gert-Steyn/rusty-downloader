use rdl_core::logging::{self, WorkerGuard};
use rdl_core::manager::{Manager as DownloadManager, ManagerConfig};
use rdl_core::{db, paths::AppPaths};

/// Long-lived services shared by all commands.
pub struct AppState {
    pub paths: AppPaths,
    pub db: db::Pool,
    pub downloads: DownloadManager,
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

        Ok(Self {
            paths,
            db,
            downloads,
            _log_guard: log_guard,
        })
    }
}
