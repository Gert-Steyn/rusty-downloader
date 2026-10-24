//! Forwards manager events to the webview in batches.
//!
//! A busy queue produces many progress events per second per download, and each
//! Tauri event is a round-trip through the webview, so events are collected for
//! a short window and sent together. Within a batch only the newest progress per
//! download is kept, and progress goes first so a status change in the same
//! batch (say, "completed") always wins.

use std::collections::HashMap;
use std::time::Duration;

use rdl_core::manager::{DownloadId, Event};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event as _;
use tokio::sync::broadcast::{self, error::RecvError};

const BATCH_WINDOW: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Default, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct DownloadEvents {
    pub events: Vec<Event>,
    /// Events were dropped because the UI fell behind; reload the full list.
    pub resync: bool,
}

#[derive(Default)]
struct Batch {
    progress: HashMap<DownloadId, Event>,
    other: Vec<Event>,
    resync: bool,
}

impl Batch {
    fn push(&mut self, event: Event) {
        match event {
            Event::Progress { download_id, .. } => {
                self.progress.insert(download_id, event);
            }
            other => self.other.push(other),
        }
    }

    fn is_empty(&self) -> bool {
        self.progress.is_empty() && self.other.is_empty() && !self.resync
    }

    fn take(&mut self) -> DownloadEvents {
        let batch = std::mem::take(self);
        let mut events: Vec<Event> = batch.progress.into_values().collect();
        events.extend(batch.other);
        DownloadEvents {
            events,
            resync: batch.resync,
        }
    }
}

pub async fn forward(app: AppHandle, mut rx: broadcast::Receiver<Event>) {
    let mut batch = Batch::default();
    loop {
        // Wait for the first event, then gather whatever else arrives in the window.
        match rx.recv().await {
            Ok(event) => batch.push(event),
            Err(RecvError::Lagged(_)) => batch.resync = true,
            Err(RecvError::Closed) => break,
        }
        let window = tokio::time::sleep(BATCH_WINDOW);
        tokio::pin!(window);
        let closed = loop {
            tokio::select! {
                _ = &mut window => break false,
                received = rx.recv() => match received {
                    Ok(event) => batch.push(event),
                    Err(RecvError::Lagged(_)) => batch.resync = true,
                    Err(RecvError::Closed) => break true,
                },
            }
        };
        if !batch.is_empty()
            && let Err(e) = batch.take().emit(&app)
        {
            tracing::warn!(error = %e, "could not emit download events");
        }
        if closed {
            break;
        }
    }
    tracing::debug!("event forwarder stopped");
}

#[cfg(test)]
mod tests {
    use rdl_core::manager::Status;

    use super::*;

    fn progress(id: DownloadId, downloaded: u64) -> Event {
        Event::Progress {
            download_id: id,
            downloaded,
            total: Some(100),
            speed_bps: 1,
            connections: 1,
        }
    }

    #[test]
    fn batch_keeps_latest_progress_and_puts_it_first() {
        let mut batch = Batch::default();
        batch.push(progress(1, 10));
        batch.push(Event::StatusChanged {
            download_id: 1,
            status: Status::Completed,
            error: None,
        });
        batch.push(progress(1, 100));

        let out = batch.take();

        assert_eq!(out.events.len(), 2);
        assert_eq!(out.events[0], progress(1, 100));
        assert!(matches!(out.events[1], Event::StatusChanged { .. }));
        assert!(batch.is_empty());
    }
}
