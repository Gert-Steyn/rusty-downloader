//! What happens when a queue has finished: nothing, sleep, hibernate, shut
//! down, or close the app (each queue's "When finished" on the Queues page).
//!
//! Anything but "nothing" first counts down for a minute: the window comes
//! to the front with the countdown and a Cancel button, and a Windows
//! notification offers Cancel too, for when nobody's at the screen to see
//! the window. Only one countdown runs at a time. Shutting down saves the
//! downloads' state first, as closing the app does. Sleeping leaves other
//! queues' downloads to reconnect after waking.
//!
//! `SetSuspendState` is a direct Windows call, hence the `unsafe`.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::time::Duration;

use rdl_core::manager::{Event, OnComplete, QueueId};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event as _;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

/// How long there is to cancel.
const COUNTDOWN: Duration = Duration::from_secs(60);

/// A countdown in progress, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingAction {
    pub action: OnComplete,
    pub queue_name: String,
    /// Whole seconds left when this was sent.
    pub seconds_left: u32,
}

/// A countdown started (`pending` set) or ended (`None`: done or cancelled).
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct OnCompleteCountdown {
    pub pending: Option<PendingAction>,
}

/// The countdown in progress, if any.
#[derive(Default)]
pub struct Countdown(Mutex<Option<(PendingAction, tokio::time::Instant, CancellationToken)>>);

impl Countdown {
    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, Option<(PendingAction, tokio::time::Instant, CancellationToken)>>
    {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn pending(&self) -> Option<PendingAction> {
        self.lock().as_ref().map(|(p, deadline, _)| PendingAction {
            seconds_left: deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .as_secs() as u32,
            ..p.clone()
        })
    }
}

/// Settings that tests and manual checks use. Debug builds only: release
/// builds always count down a minute and really do it.
fn countdown_length() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(secs) = std::env::var("RDL_ON_COMPLETE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return Duration::from_secs(secs);
    }
    COUNTDOWN
}

fn dry_run() -> bool {
    cfg!(debug_assertions) && std::env::var_os("RDL_ON_COMPLETE_DRY_RUN").is_some()
}

pub fn label(action: OnComplete) -> &'static str {
    match action {
        OnComplete::None => "Do nothing",
        OnComplete::Sleep => "Sleep",
        OnComplete::Hibernate => "Hibernate",
        OnComplete::Shutdown => "Shut down",
        OnComplete::Exit => "Close Rusty Downloader",
    }
}

/// Wait for queues to finish and start their countdowns.
pub async fn watch(app: AppHandle, mut events: broadcast::Receiver<Event>) {
    loop {
        match events.recv().await {
            Ok(Event::QueueFinished {
                queue_id,
                on_complete,
            }) if on_complete != OnComplete::None => start(&app, queue_id, on_complete).await,
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
    }
}

async fn start(app: &AppHandle, queue_id: QueueId, action: OnComplete) {
    let state = app.state::<AppState>();
    let countdown = app.state::<Countdown>();
    if countdown.lock().is_some() {
        tracing::info!(
            queue = queue_id,
            ?action,
            "a countdown is already running; ignoring"
        );
        return;
    }
    let queue_name = state
        .downloads
        .list_queues()
        .await
        .ok()
        .and_then(|qs| qs.into_iter().find(|q| q.id == queue_id))
        .map(|q| q.name)
        .unwrap_or_else(|| "A queue".into());
    let length = countdown_length();
    let pending = PendingAction {
        action,
        queue_name: queue_name.clone(),
        seconds_left: length.as_secs() as u32,
    };
    let cancel = CancellationToken::new();
    *countdown.lock() = Some((
        pending.clone(),
        tokio::time::Instant::now() + length,
        cancel.clone(),
    ));
    tracing::info!(
        queue = queue_id,
        ?action,
        seconds = length.as_secs(),
        "queue finished; counting down"
    );
    report(app, Some(pending));
    crate::tray::show_main(app);
    crate::notify::countdown(
        app,
        format!(
            "{}: {} in {} s",
            queue_name,
            label(action),
            length.as_secs()
        ),
        "The queue has finished. Cancel to stay as you are.".into(),
    );

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!(?action, "countdown cancelled");
            }
            _ = tokio::time::sleep(length) => {
                app.state::<Countdown>().lock().take();
                report(&app, None);
                perform(&app, action).await;
            }
        }
    });
}

fn report(app: &AppHandle, pending: Option<PendingAction>) {
    if let Err(e) = (OnCompleteCountdown { pending }).emit(app) {
        tracing::warn!(error = %e, "could not report the countdown");
    }
}

/// Stop the countdown, from the app or the notification.
pub fn cancel(app: &AppHandle) -> bool {
    let Some((_, _, token)) = app.state::<Countdown>().lock().take() else {
        return false;
    };
    token.cancel();
    report(app, None);
    true
}

async fn perform(app: &AppHandle, action: OnComplete) {
    if dry_run() && action != OnComplete::Exit {
        tracing::warn!(
            ?action,
            "dry run: would {} now",
            label(action).to_lowercase()
        );
        return;
    }
    tracing::info!(?action, "queue finished: acting");
    match action {
        OnComplete::None => {}
        OnComplete::Exit => app.exit(0),
        OnComplete::Shutdown => {
            // Save every download's state while the app still can.
            app.state::<AppState>().downloads.shutdown().await;
            let started = std::process::Command::new("shutdown")
                .args(["/s", "/t", "0"])
                .spawn();
            match started {
                Ok(_) => app.exit(0),
                Err(e) => tracing::error!(error = %e, "could not shut down"),
            }
        }
        OnComplete::Sleep | OnComplete::Hibernate => {
            if let Err(e) = suspend(action == OnComplete::Hibernate) {
                tracing::error!(error = %e, ?action, "could not suspend");
                crate::notify::message(
                    app,
                    format!("Couldn't {}", label(action).to_lowercase()),
                    e.to_string(),
                );
            }
        }
    }
}

/// Sleep, or hibernate (which fails when hibernation is switched off).
fn suspend(hibernate: bool) -> std::io::Result<()> {
    use windows_sys::Win32::System::Power::SetSuspendState;
    // SAFETY: plain call with by-value arguments.
    let ok = unsafe { SetSuspendState(hibernate, false, false) };
    if !ok {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The countdown in progress, for a page that opens during one.
#[tauri::command]
#[specta::specta]
pub fn on_complete_pending(countdown: State<'_, Countdown>) -> Option<PendingAction> {
    countdown.pending()
}

/// Cancel the countdown; returns whether one was running.
#[tauri::command]
#[specta::specta]
pub fn cancel_on_complete(app: AppHandle) -> bool {
    cancel(&app)
}
