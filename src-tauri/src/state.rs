use pydl_core::logging::{self, WorkerGuard};
use pydl_core::{db, paths::AppPaths};

/// Long-lived services shared by all commands.
pub struct AppState {
    pub paths: AppPaths,
    pub db: db::Pool,
    _log_guard: WorkerGuard,
}

impl AppState {
    pub async fn init() -> pydl_core::Result<Self> {
        let paths = AppPaths::resolve()?;
        paths.ensure_dirs()?;
        let log_guard = logging::init(&paths.logs_dir)?;

        tracing::info!(
            version = pydl_core::VERSION,
            portable = paths.portable,
            data_dir = %paths.data_dir.display(),
            "starting win-pydl"
        );

        let db = db::open(&paths.db_file).await?;
        tracing::info!(schema = db::schema_version(&db).await?, "database ready");

        Ok(Self {
            paths,
            db,
            _log_guard: log_guard,
        })
    }
}
