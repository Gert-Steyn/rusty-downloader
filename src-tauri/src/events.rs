//! Forwards core events to the webview in batches.
//!
//! A busy queue produces many events per second, and each Tauri event is a
//! round-trip through the webview, so events are collected for a short window
//! and sent together. Download progress is coalesced to the newest value per
//! download and placed before status changes, so a status change in the same
//! batch (say, "completed") always wins.

use std::collections::HashMap;
use std::time::Duration;

use rdl_core::linkgrabber::GrabberEvent;
use rdl_core::manager::{DownloadId, Event};
use rdl_core::tools::{ToolEvent, ToolInfo, ToolName};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use tokio::sync::broadcast::{self, error::RecvError};

const BATCH_WINDOW: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Default, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct DownloadEvents {
    pub events: Vec<Event>,
    /// Events were dropped because the UI fell behind; reload the full list.
    pub resync: bool,
}

#[derive(Debug, Clone, Default, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct GrabberEvents {
    pub events: Vec<GrabberEvent>,
    /// Events were dropped because the UI fell behind; reload the full list.
    pub resync: bool,
}

/// Tool state changes. Each carries the tool's full state, so only the newest
/// per tool is sent.
#[derive(Debug, Clone, Default, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ToolEvents {
    pub tools: Vec<ToolInfo>,
    /// Events were dropped because the UI fell behind; reload the full list.
    pub resync: bool,
}

/// Accumulates events of type `E` into one payload.
pub trait Batch<E>: Default {
    type Payload: tauri_specta::Event + Serialize + Clone;
    fn push(&mut self, event: E);
    fn lagged(&mut self);
    fn is_empty(&self) -> bool;
    fn take(&mut self) -> Self::Payload;
}

#[derive(Default)]
pub struct DownloadBatch {
    progress: HashMap<DownloadId, Event>,
    other: Vec<Event>,
    resync: bool,
}

impl Batch<Event> for DownloadBatch {
    type Payload = DownloadEvents;

    fn push(&mut self, event: Event) {
        match event {
            Event::Progress { download_id, .. } => {
                self.progress.insert(download_id, event);
            }
            other => self.other.push(other),
        }
    }

    fn lagged(&mut self) {
        self.resync = true;
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

#[derive(Default)]
pub struct GrabberBatch(GrabberEvents);

impl Batch<GrabberEvent> for GrabberBatch {
    type Payload = GrabberEvents;

    fn push(&mut self, event: GrabberEvent) {
        self.0.events.push(event);
    }

    fn lagged(&mut self) {
        self.0.resync = true;
    }

    fn is_empty(&self) -> bool {
        self.0.events.is_empty() && !self.0.resync
    }

    fn take(&mut self) -> GrabberEvents {
        std::mem::take(&mut self.0)
    }
}

#[derive(Default)]
pub struct ToolBatch {
    latest: HashMap<ToolName, ToolInfo>,
    resync: bool,
}

impl Batch<ToolEvent> for ToolBatch {
    type Payload = ToolEvents;

    fn push(&mut self, event: ToolEvent) {
        let ToolEvent::Changed { tool } = event;
        self.latest.insert(tool.name, tool);
    }

    fn lagged(&mut self) {
        self.resync = true;
    }

    fn is_empty(&self) -> bool {
        self.latest.is_empty() && !self.resync
    }

    fn take(&mut self) -> ToolEvents {
        let batch = std::mem::take(self);
        ToolEvents {
            tools: batch.latest.into_values().collect(),
            resync: batch.resync,
        }
    }
}

pub async fn forward<E: Clone, B: Batch<E>>(app: AppHandle, mut rx: broadcast::Receiver<E>) {
    use tauri_specta::Event as _;

    let mut batch = B::default();
    loop {
        // Wait for the first event, then gather whatever else arrives in the window.
        match rx.recv().await {
            Ok(event) => batch.push(event),
            Err(RecvError::Lagged(_)) => batch.lagged(),
            Err(RecvError::Closed) => break,
        }
        let window = tokio::time::sleep(BATCH_WINDOW);
        tokio::pin!(window);
        let closed = loop {
            tokio::select! {
                _ = &mut window => break false,
                received = rx.recv() => match received {
                    Ok(event) => batch.push(event),
                    Err(RecvError::Lagged(_)) => batch.lagged(),
                    Err(RecvError::Closed) => break true,
                },
            }
        };
        if !batch.is_empty()
            && let Err(e) = batch.take().emit(&app)
        {
            tracing::warn!(error = %e, "could not emit events to the UI");
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
            files: None,
        }
    }

    #[test]
    fn download_batch_keeps_latest_progress_and_puts_it_first() {
        let mut batch = DownloadBatch::default();
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

    #[test]
    fn grabber_batch_keeps_everything_in_order() {
        let mut batch = GrabberBatch::default();
        batch.push(GrabberEvent::Removed { ids: vec![1] });
        batch.push(GrabberEvent::Removed { ids: vec![2] });
        batch.lagged();

        let out = batch.take();

        assert_eq!(out.events.len(), 2);
        assert!(out.resync);
        assert!(batch.is_empty());
    }
}
