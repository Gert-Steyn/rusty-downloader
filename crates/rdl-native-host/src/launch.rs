//! Starting the app when the browser sends links and it isn't running.
//!
//! The app must outlive this host and must not hold on to anything of the
//! browser's. Rust's `Command` lets a child inherit every inheritable
//! handle, and the host's stdin/stdout are the browser's pipes: an app
//! started with them open keeps the browser waiting for the host's output
//! to end (found by testing against the real app). So they are made
//! non-inheritable first, and the app is started detached, outside the
//! browser's job where Windows allows that.
#![allow(unsafe_code)]

use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use windows_sys::Win32::Foundation::{
    HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
/// The browser's job may not allow leaving it.
const ERROR_ACCESS_DENIED: i32 = 5;

/// Start `exe --minimized` on its own.
pub fn start_app(exe: &Path) -> io::Result<()> {
    keep_std_handles_private();
    let spawn = |flags: u32| {
        Command::new(exe)
            .arg("--minimized")
            .creation_flags(flags)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    };
    let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    let child = match spawn(detached | CREATE_BREAKAWAY_FROM_JOB) {
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => spawn(detached),
        other => other,
    }?;
    drop(child); // leaves it running
    Ok(())
}

/// Stop child processes from inheriting this process's standard handles.
fn keep_std_handles_private() {
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle has no preconditions; the result is checked
        // before use, and only its inherit flag is changed.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}
