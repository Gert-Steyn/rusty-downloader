//! Registering the browser native-messaging host (`rdl-native-host.exe`,
//! next to the app) with Chrome, Edge, Brave, Chromium and Firefox.
//!
//! Done on every start, like the notification registration, so it follows
//! the app wherever it runs from: an install, a portable copy, a dev build.
//! Each browser reads a key under `HKCU\Software\…\NativeMessagingHosts`
//! whose default value is the path of a JSON manifest; the manifests live in
//! `<data>\native-host\` and name the host exe and the one extension allowed
//! to start it. If several copies of the app run, the last one started wins.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// The name the extension connects to (`chrome.runtime.connectNative`).
pub const HOST_NAME: &str = "dev.rustydownloader.host";

/// The extension's id in Chromium browsers, fixed by the public key in
/// `extension/src/manifest.json` (`node extension/build.mjs --check` makes
/// sure they agree).
const CHROMIUM_EXTENSION_ID: &str = "pnfhclaphbmimlmgkdamhmiaidjcmioh";

/// The extension's id in Firefox (`browser_specific_settings.gecko.id`).
const FIREFOX_EXTENSION_ID: &str = "extension@rustydownloader.dev";

const HOST_EXE: &str = "rdl-native-host.exe";

/// Where each browser looks, and the vendor key that shows it's used here
/// (`None`: always registered, as these are the common ones).
const CHROMIUM_BROWSERS: [(&str, Option<&str>); 4] = [
    (r"Software\Google\Chrome\NativeMessagingHosts", None),
    (r"Software\Microsoft\Edge\NativeMessagingHosts", None),
    (
        r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
        Some(r"Software\BraveSoftware"),
    ),
    (
        r"Software\Chromium\NativeMessagingHosts",
        Some(r"Software\Chromium"),
    ),
];
const FIREFOX_KEY: &str = r"Software\Mozilla\NativeMessagingHosts";

/// Register the host next to the running app. Errors only cost the browser
/// integration, so they are logged, not returned.
pub fn register(app: &AppHandle) {
    let data_dir = app.state::<AppState>().paths.data_dir.clone();
    let host = match std::env::current_exe() {
        Ok(exe) => exe.with_file_name(HOST_EXE),
        Err(e) => {
            tracing::warn!(error = %e, "can't find the app's folder; browsers can't reach it");
            return;
        }
    };
    if !host.is_file() {
        tracing::warn!(
            host = %host.display(),
            "native host missing; browsers can't reach the app (dev: cargo build -p rdl-native-host)"
        );
        return;
    }
    match write_registration(&data_dir.join("native-host"), &host) {
        Ok(browsers) => {
            tracing::info!(host = %host.display(), ?browsers, "registered for the browser extension")
        }
        Err(e) => tracing::warn!(error = %e, "could not register for the browser extension"),
    }
}

/// Undo [`register`] for the copy whose data is in `data_dir`: each
/// browser's key goes only if it points at that copy's manifests, so a
/// portable copy registered later keeps working.
pub fn unregister(data_dir: &Path) {
    let ours = data_dir.join("native-host");
    let user = windows_registry::CURRENT_USER;
    for parent in CHROMIUM_BROWSERS
        .iter()
        .map(|(key, _)| *key)
        .chain([FIREFOX_KEY])
    {
        let path = format!(r"{parent}\{HOST_NAME}");
        let points_here = user
            .open(&path)
            .and_then(|key| key.get_string(""))
            .is_ok_and(|manifest| Path::new(&manifest).starts_with(&ours));
        if points_here {
            let _ = user.remove_tree(&path);
        }
    }
}

/// Write both manifests and point each browser at its one. Returns the
/// registry keys written.
fn write_registration(dir: &Path, host: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let chromium = dir.join("chromium.json");
    let firefox = dir.join("firefox.json");
    std::fs::write(&chromium, manifest(host, Family::Chromium)).map_err(|e| e.to_string())?;
    std::fs::write(&firefox, manifest(host, Family::Firefox)).map_err(|e| e.to_string())?;

    let user = windows_registry::CURRENT_USER;
    let mut targets: Vec<(&str, &PathBuf)> = CHROMIUM_BROWSERS
        .iter()
        .filter(|(_, vendor)| vendor.is_none_or(|v| user.open(v).is_ok()))
        .map(|(key, _)| (*key, &chromium))
        .collect();
    targets.push((FIREFOX_KEY, &firefox));

    let mut written = Vec::new();
    for (parent, manifest) in targets {
        let path = format!(r"{parent}\{HOST_NAME}");
        let key = user.create(&path).map_err(|e| format!("{path}: {e}"))?;
        key.set_string("", manifest.to_string_lossy())
            .map_err(|e| format!("{path}: {e}"))?;
        written.push(path);
    }
    Ok(written)
}

#[derive(Debug, Clone, Copy)]
enum Family {
    Chromium,
    Firefox,
}

/// The host manifest for one browser family: Chromium lists allowed
/// extension origins, Firefox allowed extension ids.
fn manifest(host: &Path, family: Family) -> String {
    let mut m = serde_json::json!({
        "name": HOST_NAME,
        "description": "Rusty Downloader",
        "path": host,
        "type": "stdio",
    });
    match family {
        Family::Chromium => {
            m["allowed_origins"] =
                serde_json::json!([format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")]);
        }
        Family::Firefox => m["allowed_extensions"] = serde_json::json!([FIREFOX_EXTENSION_ID]),
    }
    serde_json::to_string_pretty(&m).expect("manifest serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_name_the_host_and_the_one_extension() {
        let host = Path::new(r"C:\Program Files\Rusty Downloader\rdl-native-host.exe");
        let chromium: serde_json::Value =
            serde_json::from_str(&manifest(host, Family::Chromium)).unwrap();
        assert_eq!(chromium["name"], HOST_NAME);
        assert_eq!(chromium["type"], "stdio");
        assert_eq!(
            chromium["path"],
            r"C:\Program Files\Rusty Downloader\rdl-native-host.exe"
        );
        assert_eq!(
            chromium["allowed_origins"],
            serde_json::json!(["chrome-extension://pnfhclaphbmimlmgkdamhmiaidjcmioh/"])
        );
        assert!(chromium.get("allowed_extensions").is_none());

        let firefox: serde_json::Value =
            serde_json::from_str(&manifest(host, Family::Firefox)).unwrap();
        assert_eq!(
            firefox["allowed_extensions"],
            serde_json::json!(["extension@rustydownloader.dev"])
        );
        assert!(firefox.get("allowed_origins").is_none());
    }

    #[test]
    fn the_host_name_is_one_browsers_accept() {
        // Lowercase letters, digits, dots and underscores; no leading or
        // trailing dot, no "..".
        assert!(
            HOST_NAME
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
        );
        assert!(!HOST_NAME.starts_with('.') && !HOST_NAME.ends_with('.'));
        assert!(!HOST_NAME.contains(".."));
    }
}
