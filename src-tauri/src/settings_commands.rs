//! Settings, History page and package-order commands.

use rdl_core::history::{self, HistoryEntry};
use rdl_core::manager::PackageId;
use rdl_core::settings::Settings;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::commands::{CommandError, CommandResult};
use crate::state::AppState;

/// The saved settings. "Start with Windows" reports what Windows actually
/// has, since the entry can be removed outside the app (Task Manager).
#[tauri::command]
#[specta::specta]
pub fn get_settings(app: AppHandle, state: State<'_, AppState>) -> Settings {
    let mut settings = state.settings().clone();
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        settings.start_with_windows = enabled;
    }
    settings
}

/// The user's Downloads folder, used when no folder is chosen in Settings.
#[tauri::command]
#[specta::specta]
pub fn system_downloads_dir(app: AppHandle) -> CommandResult<String> {
    app.path()
        .download_dir()
        .map(|p| p.display().to_string())
        .map_err(|e| CommandError(e.to_string()))
}

/// Save and apply settings; returns them as stored (trimmed, checked).
#[tauri::command]
#[specta::specta]
pub async fn set_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> CommandResult<Settings> {
    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().ok() != Some(settings.start_with_windows) {
        let changed = if settings.start_with_windows {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        changed.map_err(|e| CommandError(format!("Could not change Start with Windows: {e}")))?;
    }
    Ok(state.update_settings(settings).await?)
}

/// Newest first, at most `limit`; `search` matches name, link, folder or package.
#[tauri::command]
#[specta::specta]
pub async fn history_list(
    state: State<'_, AppState>,
    search: Option<String>,
    limit: u32,
) -> CommandResult<Vec<HistoryEntry>> {
    Ok(history::list(&state.db, search.as_deref(), limit).await?)
}

/// Forget entries; the files themselves are left alone.
#[tauri::command]
#[specta::specta]
pub async fn history_remove(state: State<'_, AppState>, ids: Vec<i64>) -> CommandResult<()> {
    Ok(history::remove(&state.db, &ids).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn history_clear(state: State<'_, AppState>) -> CommandResult<()> {
    Ok(history::clear(&state.db).await?)
}

/// Open a history entry's file (or gallery folder), or show it in Explorer.
#[tauri::command]
#[specta::specta]
pub async fn history_open(state: State<'_, AppState>, id: i64, reveal: bool) -> CommandResult<()> {
    // Looked up by id so the page can only open what the history recorded.
    let entry = history::get(&state.db, id)
        .await?
        .ok_or_else(|| CommandError("That history entry is gone.".into()))?;
    if !entry.exists {
        return Err(CommandError(format!(
            "{} is no longer there.",
            entry.path.display()
        )));
    }
    let opened = if reveal {
        tauri_plugin_opener::reveal_item_in_dir(&entry.path)
    } else {
        tauri_plugin_opener::open_path(&entry.path, None::<&str>)
    };
    opened.map_err(|e| CommandError(e.to_string()))
}

/// Move a package before another one in its queue (`None`: to the end).
#[tauri::command]
#[specta::specta]
pub async fn move_package(
    state: State<'_, AppState>,
    id: PackageId,
    before: Option<PackageId>,
) -> CommandResult<()> {
    Ok(state.downloads.move_package(id, before).await?)
}
