//! `rusty-downloader.exe --uninstall [--delete-data]`: run by the NSIS
//! uninstaller (`windows/hooks.nsh`) once the app is closed and before its
//! files go. It undoes what the app registered for the current user: the
//! browser host (when it points at this install), the notification
//! registration and Windows' "startup approved" mark. With `--delete-data`
//! it also deletes the stored passwords and the data folder; downloaded
//! files are never touched. Tauri's uninstaller itself removes the autostart
//! value, the shortcuts and the web view's data.
//!
//! A portable copy has nothing to undo here and ignores the flag.

use std::path::Path;

use rdl_core::paths::AppPaths;
use rdl_core::secrets::CredentialManager;
use rdl_core::{db, network};

use crate::native_host;

const ARG: &str = "--uninstall";
const DELETE_DATA_ARG: &str = "--delete-data";

/// `identifier` in tauri.conf.json, as notifications are registered under it.
pub const APP_ID: &str = "dev.rustydownloader.app";
/// `productName` in tauri.conf.json: the autostart value's name.
const PRODUCT_NAME: &str = "Rusty Downloader";
const STARTUP_APPROVED: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// Clean up and return `true` if the command line asks for it; the app
/// should then exit without starting.
pub fn run_if_asked() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|a| a == ARG) {
        return false;
    }
    let delete_data = args.iter().any(|a| a == DELETE_DATA_ARG);
    if let Ok(paths) = AppPaths::resolve()
        && !paths.portable
    {
        clean_up(&paths, delete_data);
    }
    true
}

/// Each step on its own: one failing doesn't stop the others.
fn clean_up(paths: &AppPaths, delete_data: bool) {
    let user = windows_registry::CURRENT_USER;
    native_host::unregister(&paths.data_dir);
    let _ = user.remove_tree(format!(r"Software\Classes\AppUserModelId\{APP_ID}"));
    if let Ok(key) = user.options().write().open(STARTUP_APPROVED) {
        let _ = key.remove_value(PRODUCT_NAME);
    }
    if delete_data {
        forget_secrets(&paths.db_file);
        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }
}

/// The passwords live in Credential Manager, not the data folder; the
/// database says which there are.
fn forget_secrets(db_file: &Path) {
    if !db_file.is_file() {
        return;
    }
    let Ok(secrets) = CredentialManager::new() else {
        return;
    };
    tauri::async_runtime::block_on(async {
        if let Ok(pool) = db::open(db_file).await {
            let _ = network::forget_secrets(&pool, secrets.as_ref()).await;
            pool.close().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_tauri_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], APP_ID);
        assert_eq!(config["productName"], PRODUCT_NAME);
    }
}
