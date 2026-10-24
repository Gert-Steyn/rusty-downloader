//! Tauri commands. Response shapes are mirrored by hand in `ui/src/lib/api.ts`.

use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// Errors cross the IPC boundary as plain messages.
#[derive(Debug, Serialize)]
pub struct CommandError(String);

impl From<pydl_core::Error> for CommandError {
    fn from(err: pydl_core::Error) -> Self {
        tracing::error!(%err, "command failed");
        Self(err.to_string())
    }
}

type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    portable: bool,
    data_dir: String,
    db_file: String,
    logs_dir: String,
    schema_version: i64,
}

#[tauri::command]
pub async fn app_info(state: State<'_, AppState>) -> CommandResult<AppInfo> {
    tracing::debug!("app_info requested");
    let paths = &state.paths;
    Ok(AppInfo {
        version: pydl_core::VERSION,
        portable: paths.portable,
        data_dir: paths.data_dir.display().to_string(),
        db_file: paths.db_file.display().to_string(),
        logs_dir: paths.logs_dir.display().to_string(),
        schema_version: pydl_core::db::schema_version(&state.db).await?,
    })
}
