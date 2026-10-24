//! LinkGrabber commands. Links arrive as raw text (pasted, dropped, typed) and
//! are extracted on the Rust side so there is one definition of "a link".

use std::path::PathBuf;

use rdl_core::linkgrabber::{GrabbedLink, LinkId, LinkSource, extract_links};
use rdl_core::manager::PackageId;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, State};

use crate::commands::{CommandError, CommandResult};
use crate::state::AppState;

#[derive(Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GrabResult {
    pub added: u32,
    /// Links that were already in the LinkGrabber.
    pub duplicates: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn grabber_list(state: State<'_, AppState>) -> CommandResult<Vec<GrabbedLink>> {
    Ok(state.grabber.list().await?)
}

/// Find links in `text` and stage them.
#[tauri::command]
#[specta::specta]
pub async fn grabber_add(
    state: State<'_, AppState>,
    text: String,
    source: LinkSource,
) -> CommandResult<GrabResult> {
    let urls = extract_links(&text);
    if urls.is_empty() {
        return Err(CommandError("No http or https links found.".into()));
    }
    let outcome = state.grabber.add(urls, source).await?;
    Ok(GrabResult {
        added: outcome.added.len() as u32,
        duplicates: outcome.duplicates as u32,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn grabber_remove(state: State<'_, AppState>, ids: Vec<LinkId>) -> CommandResult<()> {
    Ok(state.grabber.remove(&ids).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn grabber_clear(state: State<'_, AppState>) -> CommandResult<()> {
    Ok(state.grabber.clear().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn grabber_rename(
    state: State<'_, AppState>,
    id: LinkId,
    filename: String,
) -> CommandResult<()> {
    Ok(state.grabber.rename(id, &filename).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn grabber_set_package(
    state: State<'_, AppState>,
    ids: Vec<LinkId>,
    name: String,
) -> CommandResult<()> {
    Ok(state.grabber.set_package(&ids, &name).await?)
}

/// `dir` of `None` (or empty) means the default Downloads folder.
#[tauri::command]
#[specta::specta]
pub async fn grabber_set_folder(
    state: State<'_, AppState>,
    ids: Vec<LinkId>,
    dir: Option<String>,
) -> CommandResult<()> {
    let dir = dir.filter(|d| !d.trim().is_empty()).map(PathBuf::from);
    Ok(state.grabber.set_dest_dir(&ids, dir.as_deref()).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn grabber_recheck(state: State<'_, AppState>, ids: Vec<LinkId>) -> CommandResult<()> {
    Ok(state.grabber.recheck(&ids).await?)
}

/// Queue the links as downloads; returns the new package ids.
#[tauri::command]
#[specta::specta]
pub async fn grabber_confirm(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<LinkId>,
) -> CommandResult<Vec<PackageId>> {
    let default_dest = app
        .path()
        .download_dir()
        .map_err(|e| CommandError(e.to_string()))?;
    Ok(state
        .grabber
        .confirm(&ids, &default_dest, &state.downloads)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub fn get_clipboard_watch(state: State<'_, AppState>) -> bool {
    state.clipboard_watch()
}

#[tauri::command]
#[specta::specta]
pub async fn set_clipboard_watch(state: State<'_, AppState>, enabled: bool) -> CommandResult<()> {
    Ok(state.set_clipboard_watch(enabled).await?)
}
