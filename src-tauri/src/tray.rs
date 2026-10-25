//! Tray icon: a menu for the common actions, left click to open the window,
//! and a tooltip with the live download speed.

use std::collections::HashMap;
use std::time::Duration;

use rdl_core::linkgrabber::{LinkSource, extract_links};
use rdl_core::manager::{DownloadId, Event, Status};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::state::AppState;

const TRAY_ID: &str = "main";
const TITLE: &str = "Rusty Downloader";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &item("show", "Open Rusty Downloader")?,
            &PredefinedMenuItem::separator(app)?,
            &item("paste", "Add links from clipboard")?,
            &item("pause_all", "Pause all")?,
            &item("resume_all", "Resume all")?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Quit")?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(TITLE)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main(app),
        "quit" => app.exit(0),
        "paste" | "pause_all" | "resume_all" => {
            let app = app.clone();
            let id = id.to_owned();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = run_action(&app, &id).await {
                    tracing::warn!(action = %id, error = %e, "tray action failed");
                }
            });
        }
        _ => {}
    }
}

async fn run_action(app: &AppHandle, id: &str) -> rdl_core::Result<()> {
    let Some(state) = app.try_state::<AppState>() else {
        return Ok(());
    };
    match id {
        "pause_all" => state.downloads.pause_all().await,
        "resume_all" => state.downloads.resume_all().await,
        "paste" => {
            // The clipboard can be locked by another app for a moment.
            let text = clipboard_win::get_clipboard_string().unwrap_or_default();
            let urls = extract_links(&text);
            if !urls.is_empty() {
                state.grabber.add(urls, LinkSource::Clipboard).await?;
            }
            // The LinkGrabber's notice says what was added (or not).
            show_main(app);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Keep the tooltip on the running downloads and their combined speed.
pub async fn track(app: AppHandle, mut events: broadcast::Receiver<Event>) {
    let mut speeds: HashMap<DownloadId, u64> = HashMap::new();
    let mut shown = String::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(Event::Progress { download_id, speed_bps, .. }) => {
                    speeds.insert(download_id, speed_bps);
                }
                Ok(Event::StatusChanged { download_id, status, .. }) if status != Status::Active => {
                    speeds.remove(&download_id);
                }
                Ok(Event::DownloadRemoved { download_id, .. }) => {
                    speeds.remove(&download_id);
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
            _ = tick.tick() => {
                let text = tooltip(speeds.len(), speeds.values().sum());
                if text != shown
                    && let Some(tray) = app.tray_by_id(TRAY_ID)
                {
                    let _ = tray.set_tooltip(Some(&text));
                    shown = text;
                }
            }
        }
    }
}

fn tooltip(active: usize, speed: u64) -> String {
    match active {
        0 => TITLE.to_owned(),
        n => format!("{TITLE}\n{n} downloading · {}/s", bytes(speed)),
    }
}

/// `5.5 MB`, for tooltips and notifications.
pub(crate) fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_shows_activity() {
        assert_eq!(tooltip(0, 0), "Rusty Downloader");
        assert_eq!(
            tooltip(2, 5 * 1024 * 1024 + 512 * 1024),
            "Rusty Downloader\n2 downloading · 5.5 MB/s"
        );
        assert_eq!(bytes(900), "900 B");
    }
}
