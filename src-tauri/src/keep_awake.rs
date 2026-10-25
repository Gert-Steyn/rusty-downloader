//! Keep Windows from going to sleep while downloads run.
//!
//! Uses a power request (`PowerCreateRequest`, "system required"), not
//! `SetThreadExecutionState`: that one belongs to the thread that called it,
//! and async tasks move between threads. The request carries a reason that
//! `powercfg /requests` shows. The screen may still turn off; only sleep is
//! held off. Every couple of seconds the app checks whether downloads are
//! running and whether the setting is on, and holds or releases the request.
//!
//! This module calls Windows directly, hence the `unsafe`.
#![allow(unsafe_code)]

use std::io;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
};
use windows_sys::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};

use crate::state::AppState;

/// How often running downloads and the setting are checked.
const CHECK_EVERY: Duration = Duration::from_secs(2);

const REASON: &str = "Rusty Downloader is downloading";

/// `POWER_REQUEST_CONTEXT_VERSION` from winnt.h.
const POWER_REQUEST_CONTEXT_VERSION: u32 = 0;

/// Hold or release the power request as downloads start and stop, until
/// the app exits.
pub async fn watch(app: AppHandle) {
    let mut request = match PowerRequest::new(REASON) {
        Ok(request) => request,
        Err(e) => {
            tracing::warn!(error = %e, "can't keep the computer awake while downloading");
            return;
        }
    };
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
        let running = state.downloads.running_count();
        let wanted = state.settings().keep_awake && running > 0;
        if wanted != request.is_held() {
            match request.hold(wanted) {
                Ok(()) if wanted => tracing::info!(running, "keeping the computer awake"),
                Ok(()) => tracing::info!("letting the computer sleep again"),
                Err(e) => tracing::warn!(error = %e, "could not change the power request"),
            }
        }
    }
}

/// A power request: while held, Windows doesn't go to sleep on its own.
/// Released and closed when dropped.
pub struct PowerRequest {
    handle: HANDLE,
    held: bool,
}

// SAFETY: a power request handle isn't tied to the thread that created it.
unsafe impl Send for PowerRequest {}

impl PowerRequest {
    pub fn new(reason: &str) -> io::Result<Self> {
        let mut text: Vec<u16> = reason.encode_utf16().chain([0]).collect();
        let context = REASON_CONTEXT {
            Version: POWER_REQUEST_CONTEXT_VERSION,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: text.as_mut_ptr(),
            },
        };
        // SAFETY: `context` and the string it points to live for the call;
        // Windows copies the reason.
        let handle = unsafe { PowerCreateRequest(&context) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle,
            held: false,
        })
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Hold (`true`) or release (`false`) the request.
    pub fn hold(&mut self, on: bool) -> io::Result<()> {
        if on == self.held {
            return Ok(());
        }
        // SAFETY: the handle is a live power request owned by `self`.
        let ok = unsafe {
            if on {
                PowerSetRequest(self.handle, PowerRequestSystemRequired)
            } else {
                PowerClearRequest(self.handle, PowerRequestSystemRequired)
            }
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        self.held = on;
        Ok(())
    }
}

impl Drop for PowerRequest {
    fn drop(&mut self) {
        let _ = self.hold(false);
        // SAFETY: closed exactly once, here.
        unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::System::Power::{
        CallNtPowerInformation, ES_SYSTEM_REQUIRED, SystemExecutionState,
    };

    use super::*;

    /// Whether anything in the system currently asks Windows to stay awake.
    fn system_required() -> bool {
        let mut state = 0u32;
        // SAFETY: the output buffer is a u32, as SystemExecutionState returns.
        let status = unsafe {
            CallNtPowerInformation(
                SystemExecutionState,
                std::ptr::null(),
                0,
                (&mut state as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
        };
        assert_eq!(status, 0, "CallNtPowerInformation failed");
        state & ES_SYSTEM_REQUIRED != 0
    }

    #[test]
    fn holding_the_request_keeps_the_system_awake() {
        // Something else may already keep the machine awake (a video, a
        // presentation); then only the holding side can be checked.
        let before = system_required();
        eprintln!("already held awake by something else: {before}");
        let mut request = PowerRequest::new("rusty-downloader test").unwrap();
        request.hold(true).unwrap();
        assert!(request.is_held());
        assert!(system_required(), "Windows should see the request");
        request.hold(true).unwrap(); // holding twice is fine
        request.hold(false).unwrap();
        assert!(!request.is_held());
        if !before {
            assert!(!system_required(), "released, nothing should be left");
        }
        request.hold(true).unwrap();
        drop(request);
        if !before {
            assert!(!system_required(), "dropping releases it");
        }
    }
}
