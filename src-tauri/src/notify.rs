//! Windows notifications: a package finished (Open, Show in folder) or
//! downloads failed (Retry). Nothing is sent while the window has focus,
//! since the app shows it all already, and failures that arrive together
//! are summed up in one notification.
//!
//! Toasts need an AppUserModelID that Windows knows. The installer's Start
//! menu shortcut provides one, but a portable or dev build has none, so the
//! app registers its own under `HKCU\Software\Classes\AppUserModelId` with a
//! name and icon.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rdl_core::manager::{Download, DownloadId, Event, Package, Status};
use tauri::{AppHandle, Manager};
use tauri_winrt_notification::Toast;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;

use crate::state::AppState;
use crate::tray::bytes;

/// Failures this close together become one notification.
const FAILURE_WINDOW: Duration = Duration::from_secs(3);

/// Register the app so its toasts show its own name and icon. Errors only
/// cost the notifications, so they are logged, not returned.
pub fn register(app: &AppHandle) {
    let id = &app.config().identifier;
    let result = (|| -> Result<(), String> {
        let state = app.state::<AppState>();
        let icon = state.paths.data_dir.join("notification-icon.png");
        std::fs::write(&icon, include_bytes!("../icons/128x128.png")).map_err(|e| e.to_string())?;
        let key = windows_registry::CURRENT_USER
            .create(format!(r"Software\Classes\AppUserModelId\{id}"))
            .map_err(|e| e.to_string())?;
        key.set_string("DisplayName", "Rusty Downloader")
            .map_err(|e| e.to_string())?;
        key.set_string("IconUri", icon.to_string_lossy())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!(error = %e, "could not register for notifications");
    }
}

pub async fn watch(app: AppHandle, mut events: broadcast::Receiver<Event>) {
    let mut announced: HashSet<i64> = HashSet::new();
    // Galleries checked again for new files; their completion is reported by
    // `GalleryChecked`, not as a finished package.
    let mut reruns: HashSet<DownloadId> = HashSet::new();
    let mut failed: Vec<DownloadId> = Vec::new();
    let mut flush_at: Option<Instant> = None;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(Event::GalleryChecked { download_id, new_files }) => {
                    reruns.insert(download_id);
                    if let Err(e) = new_files_found(&app, download_id, new_files).await {
                        tracing::debug!(error = %e, "no new files notification");
                    }
                }
                Ok(Event::StatusChanged { download_id, status: Status::Completed, .. }) => {
                    if !reruns.remove(&download_id)
                        && let Err(e) = package_done(&app, download_id, &mut announced).await
                    {
                        tracing::debug!(error = %e, "no finished notification");
                    }
                }
                Ok(Event::StatusChanged { download_id, status: Status::Failed, .. }) => {
                    failed.push(download_id);
                    flush_at = Some(Instant::now() + FAILURE_WINDOW);
                }
                Ok(Event::PackageRemoved { package_id }) => {
                    announced.remove(&package_id);
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
            _ = tokio::time::sleep_until(flush_at.unwrap_or_else(Instant::now)), if flush_at.is_some() => {
                flush_at = None;
                let ids = std::mem::take(&mut failed);
                if let Err(e) = downloads_failed(&app, &ids).await {
                    tracing::debug!(error = %e, "no failure notification");
                }
            }
        }
    }
}

/// A queue's "when finished" countdown, with a Cancel button. Shown even
/// with the window in front: it's about the whole computer.
pub fn countdown(app: &AppHandle, title: String, body: String) {
    show(
        app,
        Note {
            title,
            body,
            buttons: vec![("Cancel".into(), "cancel-complete:".into())],
        },
    );
}

/// A plain notice, without buttons.
pub fn message(app: &AppHandle, title: String, body: String) {
    show(
        app,
        Note {
            title,
            body,
            buttons: Vec::new(),
        },
    );
}

/// Links came from the browser while the app is in the background.
pub fn links_added(app: &AppHandle, count: usize) {
    if count == 0 || in_view(app) {
        return;
    }
    let (title, body) = if count == 1 {
        (
            "1 link from the browser".into(),
            "It's in the LinkGrabber, being checked.",
        )
    } else {
        (
            format!("{count} links from the browser"),
            "They're in the LinkGrabber, being checked.",
        )
    };
    show(
        app,
        Note {
            title,
            body: body.into(),
            buttons: vec![("Review".into(), "grabber:".into())],
        },
    );
}

/// The app is in front: the user sees what happened without a toast.
fn in_view(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.is_focused().unwrap_or(false) && w.is_visible().unwrap_or(false))
}

/// A gallery checked again came down with `new_files` new files; nothing is
/// shown when there were none.
async fn new_files_found(app: &AppHandle, id: DownloadId, new_files: u32) -> Result<(), String> {
    let state = app.state::<AppState>();
    if new_files == 0 || !state.settings().notify_finished || in_view(app) {
        return Ok(());
    }
    let download = state.downloads.get(id).await.map_err(|e| e.to_string())?;
    let reveal = download
        .dest_path
        .clone()
        .ok_or_else(|| "no destination".to_owned())?;
    let plural = if new_files == 1 { "" } else { "s" };
    show(
        app,
        Note {
            title: format!("New files in {}", name(&download)),
            body: format!("{new_files} new file{plural}"),
            buttons: vec![(
                "Show in folder".into(),
                format!("reveal:{}", reveal.display()),
            )],
        },
    );
    Ok(())
}

async fn package_done(
    app: &AppHandle,
    id: DownloadId,
    announced: &mut HashSet<i64>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    if !state.settings().notify_finished || in_view(app) {
        return Ok(());
    }
    let download = state.downloads.get(id).await.map_err(|e| e.to_string())?;
    let packages = state.downloads.list().await.map_err(|e| e.to_string())?;
    let Some(package) = packages.iter().find(|p| p.id == download.package_id) else {
        return Ok(());
    };
    if package
        .downloads
        .iter()
        .any(|d| d.status != Status::Completed)
        || !announced.insert(package.id)
    {
        return Ok(());
    }
    let (title, body, open) = finished_text(package);
    let reveal = match &open {
        Some(path) => path.clone(),
        None => package.dest_dir.clone(),
    };
    let mut buttons = Vec::new();
    if let Some(path) = &open {
        buttons.push(("Open".into(), format!("open:{}", path.display())));
    }
    buttons.push((
        "Show in folder".into(),
        format!("reveal:{}", reveal.display()),
    ));
    show(
        app,
        Note {
            title,
            body,
            buttons,
        },
    );
    Ok(())
}

/// A notification's content; turned into a `Toast` where it is shown,
/// because toasts can't move between threads.
struct Note {
    title: String,
    body: String,
    /// Label and action, as `activated` reads it.
    buttons: Vec<(String, String)>,
}

/// Title, text and (for a single file) the file to open.
fn finished_text(package: &Package) -> (String, String, Option<PathBuf>) {
    let size: u64 = package.downloads.iter().filter_map(|d| d.size).sum();
    let files = package.downloads.len();
    match &package.downloads[..] {
        [one] => (
            "Download finished".into(),
            format!("{} · {}", name(one), bytes(size)),
            one.dest_path.clone(),
        ),
        _ => (
            format!("{} finished", package.name),
            format!("{files} downloads · {}", bytes(size)),
            None,
        ),
    }
}

async fn downloads_failed(app: &AppHandle, ids: &[DownloadId]) -> Result<(), String> {
    let state = app.state::<AppState>();
    if !state.settings().notify_failed || in_view(app) {
        return Ok(());
    }
    let mut failed: HashMap<DownloadId, Download> = HashMap::new();
    for id in ids {
        // Retried or removed meanwhile: nothing to report.
        if let Ok(d) = state.downloads.get(*id).await
            && d.status == Status::Failed
        {
            failed.insert(*id, d);
        }
    }
    let note = match failed.values().collect::<Vec<_>>()[..] {
        [] => return Ok(()),
        [one] => Note {
            title: format!("{} failed", name(one)),
            body: one.error.clone().unwrap_or_else(|| "Unknown error".into()),
            buttons: vec![("Retry".into(), format!("retry:{}", one.id))],
        },
        ref many => Note {
            title: format!("{} downloads failed", many.len()),
            body: many
                .iter()
                .take(3)
                .map(|d| name(d))
                .collect::<Vec<_>>()
                .join(", "),
            buttons: vec![(
                "Retry all".into(),
                format!(
                    "retry:{}",
                    many.iter()
                        .map(|d| d.id.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            )],
        },
    };
    show(app, note);
    Ok(())
}

/// Show a toast off the async runtime (WinRT calls block briefly) and act on
/// its buttons. Clicking the toast itself opens the window.
///
/// Toasts go out one at a time: two shown in the same instant (two packages
/// finishing together) collide in Windows' notification store
/// (`SQLITE_CONSTRAINT_UNIQUE`) and one is lost, so a refused one is retried.
fn show(app: &AppHandle, note: Note) {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let toast = || {
            let handler = app.clone();
            let mut toast = Toast::new(&app.config().identifier)
                .title(&note.title)
                .text1(&note.body)
                .on_activated(move |action| {
                    activated(&handler, action.as_deref());
                    Ok(())
                });
            for (label, action) in &note.buttons {
                toast = toast.add_button(label, action);
            }
            toast
        };
        let mut result = toast().show();
        if result.is_err() {
            std::thread::sleep(Duration::from_millis(250));
            result = toast().show();
        }
        if let Err(e) = result {
            tracing::warn!(error = %e, "could not show notification");
        }
        // Space them out so the next one doesn't land in the same instant.
        std::thread::sleep(Duration::from_millis(100));
    });
}

fn activated(app: &AppHandle, action: Option<&str>) {
    let Some((verb, arg)) = action.and_then(|a| a.split_once(':')) else {
        crate::tray::show_main(app);
        return;
    };
    match verb {
        "open" => {
            let _ = tauri_plugin_opener::open_path(Path::new(arg), None::<&str>);
        }
        "reveal" => {
            let _ = tauri_plugin_opener::reveal_item_in_dir(Path::new(arg));
        }
        "retry" => {
            let app = app.clone();
            let ids: Vec<DownloadId> = arg.split(',').filter_map(|i| i.parse().ok()).collect();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                for id in ids {
                    if let Err(e) = state.downloads.resume(id).await {
                        tracing::warn!(download = id, error = %e, "retry from notification failed");
                    }
                }
            });
        }
        "cancel-complete" => {
            crate::on_complete::cancel(app);
        }
        "grabber" => {
            crate::tray::show_main(app);
            if let Some(window) = app.get_webview_window("main") {
                // Navigate with the app's own link, so the router handles it.
                let _ = window.eval(r#"document.querySelector('a[href="/linkgrabber"]')?.click()"#);
            }
        }
        _ => crate::tray::show_main(app),
    }
}

/// The name a download is shown under.
fn name(d: &Download) -> String {
    d.filename
        .clone()
        .or_else(|| {
            d.dest_path
                .as_deref()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
        })
        .or_else(|| d.title.clone())
        // A download that failed before its name was known: the link's last part.
        .or_else(|| {
            url::Url::parse(&d.source_url)
                .ok()?
                .path_segments()?
                .rfind(|s| !s.is_empty())
                .map(|s| s.to_owned())
        })
        .unwrap_or_else(|| d.source_url.clone())
}

#[cfg(test)]
mod tests {
    use rdl_core::manager::Download;

    use super::*;

    fn download(url: &str) -> Download {
        Download {
            id: 1,
            package_id: 1,
            kind: "http".into(),
            source_url: url.into(),
            filename: None,
            dest_path: None,
            size: None,
            downloaded: 0,
            status: Status::Failed,
            error: None,
            ytdlp: None,
            gallerydl: None,
            torrent: None,
            files: None,
            title: None,
            speed_limit: None,
            checksum: None,
            proxy: Default::default(),
        }
    }

    #[test]
    fn unnamed_downloads_go_by_the_last_part_of_their_link() {
        assert_eq!(
            name(&download("http://x.test/files/missing.zip?a=1")),
            "missing.zip"
        );
        assert_eq!(name(&download("http://x.test/dir/")), "dir");
        assert_eq!(name(&download("http://x.test/")), "http://x.test/");
        let named = Download {
            filename: Some("chosen.zip".into()),
            ..download("http://x.test/a")
        };
        assert_eq!(name(&named), "chosen.zip");
    }
}
