//! Tauri commands. Types and signatures are exported to `ui/src/lib/bindings.ts`
//! by tauri-specta, so the UI calls them with full type checking.

use std::path::{Path, PathBuf};

use rdl_core::linkgrabber::default_package_name;
use rdl_core::manager::{
    DEFAULT_QUEUE, Download, DownloadId, NewDownload, NewPackage, Package, PackageId, Status,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use url::Url;

use crate::state::AppState;

/// Errors cross the IPC boundary as plain messages.
#[derive(Debug, Serialize, Type)]
pub struct CommandError(pub(crate) String);

impl From<rdl_core::Error> for CommandError {
    fn from(err: rdl_core::Error) -> Self {
        tracing::error!(%err, "command failed");
        Self(err.to_string())
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

pub(crate) type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    portable: bool,
    data_dir: String,
    db_file: String,
    logs_dir: String,
    schema_version: i64,
}

#[tauri::command]
#[specta::specta]
pub async fn app_info(state: State<'_, AppState>) -> CommandResult<AppInfo> {
    let paths = &state.paths;
    Ok(AppInfo {
        version: rdl_core::VERSION.to_owned(),
        portable: paths.portable,
        data_dir: paths.data_dir.display().to_string(),
        db_file: paths.db_file.display().to_string(),
        logs_dir: paths.logs_dir.display().to_string(),
        schema_version: rdl_core::db::schema_version(&state.db).await?,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn list_packages(state: State<'_, AppState>) -> CommandResult<Vec<Package>> {
    Ok(state.downloads.list().await?)
}

/// The user's Downloads folder, the default destination for new packages.
#[tauri::command]
#[specta::specta]
pub fn default_download_dir(app: AppHandle) -> CommandResult<String> {
    app.path()
        .download_dir()
        .map(|p| p.display().to_string())
        .map_err(|e| CommandError(e.to_string()))
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddLinks {
    pub urls: Vec<String>,
    /// Defaults to a name derived from the links.
    pub package_name: Option<String>,
    /// Defaults to the Downloads folder.
    pub dest_dir: Option<String>,
}

#[derive(Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddLinksResult {
    pub package_id: PackageId,
    pub added: u32,
    /// Inputs that were not valid http(s) URLs.
    pub rejected: Vec<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn add_links(
    app: AppHandle,
    state: State<'_, AppState>,
    request: AddLinks,
) -> CommandResult<AddLinksResult> {
    let (urls, rejected) = parse_links(&request.urls);
    if urls.is_empty() {
        return Err("No valid http or https links to add.".to_owned().into());
    }
    let dest_dir = match request
        .dest_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        Some(dir) => PathBuf::from(dir),
        None => app
            .path()
            .download_dir()
            .map_err(|e| CommandError(e.to_string()))?,
    };
    let name = request
        .package_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| default_package_name(&urls));

    let added = urls.len() as u32;
    let package_id = state
        .downloads
        .add_package(NewPackage {
            name,
            dest_dir,
            queue_id: DEFAULT_QUEUE,
            downloads: urls.into_iter().map(NewDownload::new).collect(),
        })
        .await?;
    Ok(AddLinksResult {
        package_id,
        added,
        rejected,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn pause_download(state: State<'_, AppState>, id: DownloadId) -> CommandResult<()> {
    Ok(state.downloads.pause(id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn resume_download(state: State<'_, AppState>, id: DownloadId) -> CommandResult<()> {
    Ok(state.downloads.resume(id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn pause_all(state: State<'_, AppState>) -> CommandResult<()> {
    Ok(state.downloads.pause_all().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn resume_all(state: State<'_, AppState>) -> CommandResult<()> {
    Ok(state.downloads.resume_all().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_download(
    state: State<'_, AppState>,
    id: DownloadId,
    delete_files: bool,
) -> CommandResult<()> {
    Ok(state.downloads.remove(id, delete_files).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_package(
    state: State<'_, AppState>,
    id: PackageId,
    delete_files: bool,
) -> CommandResult<()> {
    Ok(state.downloads.remove_package(id, delete_files).await?)
}

/// Open a finished download with its default application.
#[tauri::command]
#[specta::specta]
pub async fn open_download(state: State<'_, AppState>, id: DownloadId) -> CommandResult<()> {
    let download = state.downloads.get(id).await?;
    let path = download
        .dest_path
        .filter(|_| download.status == Status::Completed)
        .ok_or_else(|| "The download has not finished yet.".to_owned())?;
    open_path(&path)
}

/// Show a download in Explorer: the finished file, its partial file, or
/// failing both its package folder.
#[tauri::command]
#[specta::specta]
pub async fn reveal_download(state: State<'_, AppState>, id: DownloadId) -> CommandResult<()> {
    let download = state.downloads.get(id).await?;
    if let Some(path) = existing_file(&download) {
        return tauri_plugin_opener::reveal_item_in_dir(&path)
            .map_err(|e| CommandError(e.to_string()));
    }
    let folder = package_folder(&state, download.package_id).await?;
    open_path(&folder)
}

#[tauri::command]
#[specta::specta]
pub async fn open_package_folder(state: State<'_, AppState>, id: PackageId) -> CommandResult<()> {
    let folder = package_folder(&state, id).await?;
    open_path(&folder)
}

fn existing_file(download: &Download) -> Option<PathBuf> {
    let dest = download.dest_path.as_ref()?;
    if dest.exists() {
        return Some(dest.clone());
    }
    let mut part = dest.clone().into_os_string();
    part.push(".rdl.part");
    Some(PathBuf::from(part)).filter(|p| p.exists())
}

async fn package_folder(state: &AppState, id: PackageId) -> CommandResult<PathBuf> {
    let packages = state.downloads.list().await?;
    let package = packages
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("Package {id} not found."))?;
    std::fs::create_dir_all(&package.dest_dir).map_err(|e| CommandError(e.to_string()))?;
    Ok(package.dest_dir)
}

fn open_path(path: &Path) -> CommandResult<()> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| CommandError(e.to_string()))
}

/// Split input into valid http(s) URLs and everything else (trimmed, blanks dropped).
fn parse_links(inputs: &[String]) -> (Vec<Url>, Vec<String>) {
    let mut urls = Vec::new();
    let mut rejected = Vec::new();
    for input in inputs.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match Url::parse(input) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => urls.push(url),
            _ => rejected.push(input.to_owned()),
        }
    }
    (urls, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_split_into_valid_and_rejected() {
        let inputs = [
            " https://example.com/a.zip ".to_owned(),
            "".to_owned(),
            "ftp://example.com/b".to_owned(),
            "not a url".to_owned(),
            "http://example.com/c".to_owned(),
        ];
        let (urls, rejected) = parse_links(&inputs);
        assert_eq!(urls.len(), 2);
        assert_eq!(rejected, ["ftp://example.com/b", "not a url"]);
    }
}
