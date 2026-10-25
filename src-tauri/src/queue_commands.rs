//! Queues page commands.

use rdl_core::manager::{DEFAULT_QUEUE, PackageId, Queue, QueueChanges, QueueId};
use tauri::State;

use crate::commands::CommandResult;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_queues(state: State<'_, AppState>) -> CommandResult<Vec<Queue>> {
    Ok(state.downloads.list_queues().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn add_queue(state: State<'_, AppState>, name: String) -> CommandResult<QueueId> {
    Ok(state.downloads.add_queue(&name).await?)
}

/// The main queue's "downloads at once" is also the setting of that name,
/// so the two stay the same whichever page changes it.
#[tauri::command]
#[specta::specta]
pub async fn update_queue(
    state: State<'_, AppState>,
    id: QueueId,
    changes: QueueChanges,
) -> CommandResult<()> {
    let max_active = changes.max_active;
    state.downloads.update_queue(id, changes).await?;
    if id == DEFAULT_QUEUE && state.settings().max_active != max_active {
        let mut settings = state.settings().clone();
        settings.max_active = max_active;
        state.update_settings(settings).await?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn remove_queue(state: State<'_, AppState>, id: QueueId) -> CommandResult<()> {
    Ok(state.downloads.remove_queue(id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn move_package_to_queue(
    state: State<'_, AppState>,
    package_id: PackageId,
    queue: QueueId,
) -> CommandResult<()> {
    Ok(state
        .downloads
        .move_package_to_queue(package_id, queue)
        .await?)
}
