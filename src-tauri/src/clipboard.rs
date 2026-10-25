//! Clipboard watcher: links copied anywhere land in the LinkGrabber, or with
//! "ask first" on, are offered with an Add button.
//!
//! Polls `GetClipboardSequenceNumber`, which is a cheap counter read, and only
//! opens the clipboard when it changes. Whatever is on the clipboard when the
//! app starts (or when watching is switched on) is ignored, so old links don't
//! reappear. Links matching the ignore patterns in Settings are dropped.

use std::time::Duration;

use rdl_core::linkgrabber::{LinkSource, extract_links};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::state::AppState;

const POLL: Duration = Duration::from_millis(400);

/// Links were copied while "ask first" is on. `text` holds them one per line,
/// ready for `grabber_add`.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardOffer {
    pub text: String,
    pub count: u32,
}

pub async fn watch(app: AppHandle) {
    let mut last_seq = clipboard_win::raw::seq_num();
    let mut last_text = String::new();
    loop {
        tokio::time::sleep(POLL).await;
        let seq = clipboard_win::raw::seq_num();
        if seq == last_seq {
            continue;
        }
        last_seq = seq;

        let Some(state) = app.try_state::<AppState>() else {
            continue;
        };
        let settings = state.settings().clone();
        if !settings.clipboard_watch {
            continue;
        }
        // The clipboard can be locked by another app; just try again next change.
        let Ok(text) = clipboard_win::get_clipboard_string() else {
            continue;
        };
        if text == last_text {
            continue;
        }
        let urls: Vec<_> = extract_links(&text)
            .into_iter()
            .filter(|u| !settings.clipboard_ignores(u.as_str()))
            .collect();
        last_text = text;
        if urls.is_empty() {
            continue;
        }
        if settings.clipboard_ask {
            let offer = ClipboardOffer {
                count: urls.len() as u32,
                text: urls
                    .iter()
                    .map(|u| u.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            if let Err(e) = offer.emit(&app) {
                tracing::warn!(error = %e, "could not offer clipboard links");
            }
            continue;
        }
        match state.grabber.add(urls, LinkSource::Clipboard).await {
            Ok(outcome) if !outcome.added.is_empty() => {
                tracing::debug!(added = outcome.added.len(), "links taken from clipboard");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "could not add clipboard links"),
        }
    }
}
