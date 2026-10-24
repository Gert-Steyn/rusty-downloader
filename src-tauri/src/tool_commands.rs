//! Tools page commands: versions, installs, updates and rollback. Long-running
//! ones (install, check) also report progress through `ToolEvents`.

use rdl_core::tools::{ToolChannel, ToolInfo, ToolName};
use tauri::State;

use crate::commands::{CommandError, CommandResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn tools_list(state: State<'_, AppState>) -> CommandResult<Vec<ToolInfo>> {
    Ok(state.tools.list().await?)
}

/// Install the tool, or update it to the newest release.
#[tauri::command]
#[specta::specta]
pub async fn tool_install(state: State<'_, AppState>, name: ToolName) -> CommandResult<ToolInfo> {
    Ok(state.tools.install(name).await?)
}

/// Look for a newer release without installing it.
#[tauri::command]
#[specta::specta]
pub async fn tool_check(state: State<'_, AppState>, name: ToolName) -> CommandResult<ToolInfo> {
    Ok(state.tools.check(name).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tool_rollback(state: State<'_, AppState>, name: ToolName) -> CommandResult<ToolInfo> {
    Ok(state.tools.rollback(name).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tool_set_channel(
    state: State<'_, AppState>,
    name: ToolName,
    channel: ToolChannel,
) -> CommandResult<ToolInfo> {
    Ok(state.tools.set_channel(name, channel).await?)
}

#[tauri::command]
#[specta::specta]
pub fn open_tools_folder(state: State<'_, AppState>) -> CommandResult<()> {
    let dir = &state.paths.tools_dir;
    std::fs::create_dir_all(dir).map_err(|e| CommandError(e.to_string()))?;
    tauri_plugin_opener::open_path(dir, None::<&str>).map_err(|e| CommandError(e.to_string()))
}
