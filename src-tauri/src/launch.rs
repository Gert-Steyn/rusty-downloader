//! Magnet links and `.torrent` files handed to the app from outside: Windows
//! starts the app with the link or file on its command line (once the app is
//! the default), and a second launch hands over to the running copy.
//!
//! Each item goes to the LinkGrabber (magnets are checked there, and a
//! `.torrent` is copied into the torrent folder), then the main window comes
//! up on the LinkGrabber. Other arguments (flags, the program's own path) are
//! ignored.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rdl_core::linkgrabber::LinkSource;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use url::Url;

use crate::state::AppState;
use crate::tray;

/// Where the window goes after a launch with links.
pub const LINKGRABBER: &str = "/linkgrabber";

/// Something the command line asked the app to take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    Magnet(Url),
    TorrentFile(PathBuf),
}

/// The items on a command line (the first argument is the program itself, which is ignored).
/// A `.torrent` must exist as a file now, so a stale association doesn't add nothing silently.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Vec<Incoming> {
    args.into_iter()
        .skip(1)
        .filter_map(|arg| {
            let arg = arg.trim();
            if arg.is_empty() || arg.starts_with("--") {
                None
            } else if is_magnet(arg) {
                // A magnet has its parameters in the query; "magnet:" alone is nothing.
                Url::parse(arg)
                    .ok()
                    .filter(|url| url.scheme() == "magnet" && url.query().is_some())
                    .map(Incoming::Magnet)
            } else {
                let path = Path::new(arg);
                (is_torrent_name(path) && path.is_file())
                    .then(|| Incoming::TorrentFile(path.to_path_buf()))
            }
        })
        .collect()
}

fn is_magnet(arg: &str) -> bool {
    arg.get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("magnet:"))
}

fn is_torrent_name(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
}

/// Asked of the UI on the next start, for a launch that happened before the
/// window's listeners were up.
static PENDING_PAGE: Mutex<Option<String>> = Mutex::new(None);

/// Tells the UI which page to show after a launch.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct OpenPage {
    pub path: String,
}

/// Stage the items in the LinkGrabber and bring the window up on it.
pub async fn open(app: AppHandle, items: Vec<Incoming>) {
    if items.is_empty() {
        return;
    }
    let count = items.len();
    {
        let state = app.state::<AppState>();
        for item in items {
            let added = match item {
                Incoming::Magnet(url) => state
                    .grabber
                    .add(vec![url], LinkSource::Shell)
                    .await
                    .map(|outcome| outcome.added.len()),
                Incoming::TorrentFile(path) => state
                    .grabber
                    .add_torrent_path(&path, LinkSource::Shell)
                    .await
                    .map(|outcome| outcome.added.len()),
            };
            if let Err(e) = added {
                tracing::warn!(error = %e, "could not take a link from the command line");
            }
        }
    }
    tracing::info!(count, "links taken from the command line");
    tray::show_main(&app);
    if let Ok(mut pending) = PENDING_PAGE.lock() {
        *pending = Some(LINKGRABBER.to_owned());
    }
    if let Err(e) = (OpenPage {
        path: LINKGRABBER.to_owned(),
    })
    .emit(&app)
    {
        tracing::warn!(error = %e, "could not tell the window to open the LinkGrabber");
    }
}

/// The page to open, if a launch asked for one and the window hasn't taken it yet.
#[tauri::command]
#[specta::specta]
pub fn take_pending_page() -> Option<String> {
    PENDING_PAGE.lock().ok()?.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn magnets_and_torrent_files_are_taken_and_flags_ignored() {
        let dir = std::env::temp_dir().join(format!("rdl-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let torrent = dir.join("one.torrent");
        let upper = dir.join("two.TORRENT");
        std::fs::write(&torrent, b"d4:infod e e").unwrap();
        std::fs::write(&upper, b"d4:infod e e").unwrap();
        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Test";

        let line = vec![
            r"C:\Program Files\Rusty Downloader\rusty-downloader.exe".to_owned(),
            "--minimized".to_owned(),
            magnet.to_owned(),
            torrent.display().to_string(),
            "  ".to_owned(),
            r"C:\missing\gone.torrent".to_owned(),
            "https://example.com/a.zip".to_owned(),
            "MAGNET:?xt=urn:btih:ABC".to_owned(),
            upper.display().to_string(),
        ];
        let items = parse_args(line);
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            items,
            vec![
                Incoming::Magnet(Url::parse(magnet).unwrap()),
                Incoming::TorrentFile(torrent.clone()),
                Incoming::Magnet(Url::parse("MAGNET:?xt=urn:btih:ABC").unwrap()),
                Incoming::TorrentFile(upper),
            ]
        );
    }

    #[test]
    fn nothing_on_the_command_line_means_nothing_to_take() {
        assert!(parse_args(args(&["app.exe"])).is_empty());
        assert!(parse_args(Vec::new()).is_empty());
        assert!(parse_args(args(&["app.exe", "--minimized", "magnet:"])).is_empty());
    }
}
