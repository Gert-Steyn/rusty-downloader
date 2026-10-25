//! Who is on the other end of the pipe?
//!
//! Any program can create a pipe with the app's name before the app does
//! (pipe names are global, not per user). Cookies must only go to the real
//! app, so before anything is sent the server's process is checked: it must
//! run the expected `rusty-downloader.exe` and belong to the same Windows
//! user as this host.
//!
//! This is the only module that calls Windows directly, hence the `unsafe`.
#![allow(unsafe_code)]

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

/// Check that `pipe`'s server is `expected_exe`, run by the current user.
/// The error says why not, for the extension to show.
pub fn check_server(pipe: &impl AsRawHandle, expected_exe: &Path) -> Result<(), String> {
    let pid = server_pid(pipe).map_err(|e| format!("could not tell who serves the pipe: {e}"))?;
    let process = open_process(pid)
        .map_err(|e| format!("could not inspect the pipe's server (process {pid}): {e}"))?;

    let actual = image_path(&process)
        .map_err(|e| format!("could not read the pipe server's program: {e}"))?;
    if !same_file(&actual, expected_exe) {
        return Err(format!(
            "the pipe is served by {} instead of {}",
            actual.display(),
            expected_exe.display()
        ));
    }

    let theirs =
        TokenUserInfo::of(process.0).map_err(|e| format!("could not read its user: {e}"))?;
    // SAFETY: the pseudo handle needs no closing and is always valid.
    let ours = TokenUserInfo::of(unsafe { GetCurrentProcess() })
        .map_err(|e| format!("could not read this user: {e}"))?;
    // SAFETY: both SIDs point into buffers that live until the end of this call.
    if unsafe { EqualSid(theirs.sid(), ours.sid()) } == 0 {
        return Err("the pipe is served by another Windows user's program".into());
    }
    Ok(())
}

/// Two paths name the same file: compared after resolving them, ignoring
/// case as NTFS does.
fn same_file(a: &Path, b: &Path) -> bool {
    let resolve = |p: &Path| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_owned())
            .to_string_lossy()
            .to_lowercase()
    };
    resolve(a) == resolve(b)
}

/// A process or token handle, closed when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by us and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

fn server_pid(pipe: &impl AsRawHandle) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: the handle belongs to an open pipe client for this call's duration.
    let ok = unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

fn open_process(pid: u32) -> io::Result<Owned> {
    // SAFETY: plain call; a null result is checked.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(Owned(handle))
}

fn image_path(process: &Owned) -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 32 * 1024];
    let mut len = buffer.len() as u32;
    // SAFETY: `buffer` holds `len` UTF-16 units; the call writes at most that many.
    let ok = unsafe {
        QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len)
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..len as usize])))
}

/// A process's `TOKEN_USER`, in a buffer aligned for it.
struct TokenUserInfo(Vec<u64>);

impl TokenUserInfo {
    fn of(process: HANDLE) -> io::Result<Self> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: `token` receives a handle that `Owned` closes.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Owned(token);
        let mut needed = 0u32;
        // SAFETY: asking for the size only; no buffer is written.
        unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `buffer` has room for `needed` bytes and is 8-byte aligned.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buffer))
    }

    fn sid(&self) -> PSID {
        // SAFETY: the buffer was filled by GetTokenInformation(TokenUser), so
        // it starts with a TOKEN_USER whose SID points into the same buffer.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_compare_resolved_and_without_case() {
        let exe = std::env::current_exe().unwrap();
        let upper = PathBuf::from(exe.to_string_lossy().to_uppercase());
        assert!(same_file(&exe, &upper));
        let dotted = exe
            .parent()
            .unwrap()
            .join(".")
            .join(exe.file_name().unwrap());
        assert!(same_file(&exe, &dotted));
        assert!(!same_file(&exe, &exe.with_file_name("other.exe")));
    }

    #[test]
    fn reads_this_process() {
        // SAFETY: the pseudo handle is always valid.
        let me = TokenUserInfo::of(unsafe { GetCurrentProcess() }).unwrap();
        let again = TokenUserInfo::of(unsafe { GetCurrentProcess() }).unwrap();
        assert_ne!(unsafe { EqualSid(me.sid(), again.sid()) }, 0);

        let process = open_process(std::process::id()).unwrap();
        let path = image_path(&process).unwrap();
        assert!(same_file(&path, &std::env::current_exe().unwrap()));
    }
}
