//! The speed limit in force: the status bar's switch, or a scheduled rule.
//!
//! Settings changes apply it at once (`AppState::apply`). This task follows
//! the clock, so scheduled limits begin and end on time, and tells the UI
//! whenever what's in force changes.

use std::time::Duration;

use rdl_core::speed::ActiveLimit;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::commands::CommandResult;
use crate::state::AppState;

/// How often the schedule is checked. Windows start and end on the minute,
/// so a rule takes effect within this of its time.
const CHECK_EVERY: Duration = Duration::from_secs(15);

/// What limits the speed now changed.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SpeedLimitStatus {
    pub active: ActiveLimit,
}

pub async fn follow_schedule(app: AppHandle) {
    let mut shown: Option<ActiveLimit> = None;
    let mut tick = tokio::time::interval(CHECK_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(state) = app.try_state::<AppState>() else {
            continue;
        };
        if state.shutdown.is_cancelled() {
            break;
        }
        let active = state.settings().active_speed_limit_now();
        state.downloads.set_speed_limit(active.limit);
        if shown.as_ref() != Some(&active) {
            if let Err(e) = (SpeedLimitStatus {
                active: active.clone(),
            })
            .emit(&app)
            {
                tracing::warn!(error = %e, "could not report the speed limit");
            }
            shown = Some(active);
        }
    }
}

/// What limits the speed now.
#[tauri::command]
#[specta::specta]
pub fn speed_limit_status(state: State<'_, AppState>) -> ActiveLimit {
    state.settings().active_speed_limit_now()
}

/// The status bar's switch. Returns what limits the speed afterwards.
#[tauri::command]
#[specta::specta]
pub async fn set_speed_limit_on(
    app: AppHandle,
    state: State<'_, AppState>,
    on: bool,
) -> CommandResult<ActiveLimit> {
    let mut settings = state.settings().clone();
    settings.speed_limit_on = on;
    let saved = state.update_settings(settings).await?;
    let active = saved.active_speed_limit_now();
    let _ = (SpeedLimitStatus {
        active: active.clone(),
    })
    .emit(&app);
    Ok(active)
}
