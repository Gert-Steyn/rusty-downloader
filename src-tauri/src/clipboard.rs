//! Clipboard watcher: links copied anywhere land in the LinkGrabber.
//!
//! Polls `GetClipboardSequenceNumber`, which is a cheap counter read, and only
//! opens the clipboard when it changes. Whatever is on the clipboard when the
//! app starts (or when watching is switched on) is ignored, so old links don't
//! reappear.

use std::time::Duration;

use rdl_core::linkgrabber::{LinkSource, extract_links};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

const POLL: Duration = Duration::from_millis(400);

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
        if !state.clipboard_watch() {
            continue;
        }
        // The clipboard can be locked by another app; just try again next change.
        let Ok(text) = clipboard_win::get_clipboard_string() else {
            continue;
        };
        if text == last_text {
            continue;
        }
        let urls = extract_links(&text);
        last_text = text;
        if urls.is_empty() {
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
