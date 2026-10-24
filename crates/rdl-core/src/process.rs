//! Running external tools (yt-dlp, ffmpeg).
//!
//! Every process gets no console window, UTF-8 output, and on Windows its own
//! job object with kill-on-close. The job matters: yt-dlp.exe is a PyInstaller
//! bootloader that starts a second process, and yt-dlp starts ffmpeg, so
//! killing just the child we spawned would leave the real work running.
//! Dropping the job (on cancel, or when the app dies) ends the whole tree.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

/// One line of output, without its line ending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Out(String),
    Err(String),
}

/// A tool command with the standard environment applied.
pub fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// A running tool. Dropping it kills the process tree.
pub struct Running {
    child: Child,
    lines: mpsc::Receiver<Line>,
    #[cfg(windows)]
    job: Option<win32job::Job>,
}

impl Running {
    pub fn spawn(mut cmd: Command) -> Result<Self> {
        let program = cmd.as_std().get_program().to_owned();
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Tool(format!("could not start {}: {e}", display(&program))))?;
        #[cfg(windows)]
        let job = assign_job(&child);

        let (tx, lines) = mpsc::channel(256);
        if let Some(out) = child.stdout.take() {
            tokio::spawn(read_lines(out, tx.clone(), Line::Out));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(read_lines(err, tx, Line::Err));
        }
        Ok(Self {
            child,
            lines,
            #[cfg(windows)]
            job,
        })
    }

    /// Next line from stdout or stderr; `None` once both are closed.
    pub async fn next_line(&mut self) -> Option<Line> {
        self.lines.recv().await
    }

    pub async fn wait(&mut self) -> Result<ExitStatus> {
        Ok(self.child.wait().await?)
    }

    /// Kill the process and everything it started, and reap it.
    pub async fn kill(&mut self) {
        #[cfg(windows)]
        drop(self.job.take());
        let _ = self.child.kill().await;
    }
}

#[cfg(windows)]
fn assign_job(child: &Child) -> Option<win32job::Job> {
    // A child that starts its own children in the instant before this runs
    // would escape the job; PyInstaller unpacks first, so in practice it doesn't.
    let handle = child.raw_handle()? as isize;
    let mut limits = win32job::ExtendedLimitInfo::new();
    limits.limit_kill_on_job_close();
    let job = win32job::Job::create_with_limit_info(&limits)
        .and_then(|job| job.assign_process(handle).map(|()| job));
    match job {
        Ok(job) => Some(job),
        Err(e) => {
            tracing::warn!(error = %e, "could not put tool process in a job object");
            None
        }
    }
}

async fn read_lines<R: AsyncRead + Unpin>(
    reader: R,
    tx: mpsc::Sender<Line>,
    wrap: fn(String) -> Line,
) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let text = String::from_utf8_lossy(&buf);
                let line = text.trim_end_matches(['\r', '\n']).to_owned();
                if tx.send(wrap(line)).await.is_err() {
                    break;
                }
            }
        }
    }
}

/// Everything a finished command printed.
#[derive(Debug, Clone)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// The last `ERROR:` line yt-dlp-style tools print, else the last stderr line.
    pub fn error_message(&self) -> String {
        error_message(self.stderr.lines()).unwrap_or_else(|| format!("exited with {}", self.status))
    }
}

pub(crate) fn error_message<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> Option<String> {
    let lines: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    lines
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("ERROR:"))
        .or_else(|| lines.last().copied())
        .map(|l| l.trim().to_owned())
}

/// Run `cmd` to completion, collecting its output. Cancelling or timing out
/// kills it.
pub async fn output(cmd: Command, timeout: Duration, cancel: &CancellationToken) -> Result<Output> {
    let program = display(cmd.as_std().get_program());
    let mut running = Running::spawn(cmd)?;
    let mut stdout = String::new();
    let mut stderr = String::new();
    let collect = async {
        while let Some(line) = running.next_line().await {
            let (buf, text) = match line {
                Line::Out(t) => (&mut stdout, t),
                Line::Err(t) => (&mut stderr, t),
            };
            buf.push_str(&text);
            buf.push('\n');
        }
    };
    let finished = tokio::select! {
        _ = cancel.cancelled() => false,
        r = tokio::time::timeout(timeout, collect) => r.is_ok(),
    };
    if !finished {
        running.kill().await;
        return Err(if cancel.is_cancelled() {
            Error::Canceled
        } else {
            Error::Tool(format!(
                "{program} did not finish within {}s",
                timeout.as_secs()
            ))
        });
    }
    let status = running.wait().await?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn display(program: &OsStr) -> String {
    Path::new(program)
        .file_name()
        .unwrap_or(program)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_prefers_error_lines() {
        let stderr =
            "WARNING: slow\nERROR: [youtube] abc: Video unavailable\nsome trailing noise\n";
        assert_eq!(
            error_message(stderr.lines()).as_deref(),
            Some("[youtube] abc: Video unavailable")
        );
        assert_eq!(
            error_message("only this\n\n".lines()).as_deref(),
            Some("only this")
        );
        assert_eq!(error_message("".lines()), None);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn captures_output_of_a_real_process() {
        let mut cmd = command(Path::new("cmd.exe"));
        cmd.args(["/C", "echo out & echo err 1>&2"]);
        let out = output(cmd, Duration::from_secs(10), &CancellationToken::new())
            .await
            .unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.stderr.trim(), "err");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cancel_kills_a_long_running_process() {
        let mut cmd = command(Path::new("cmd.exe"));
        // ping waits ~1 s per extra echo; 30 is far longer than the test.
        cmd.args(["/C", "ping -n 30 127.0.0.1 >NUL"]);
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            trigger.cancel();
        });
        let started = std::time::Instant::now();
        let result = output(cmd, Duration::from_secs(60), &cancel).await;
        assert!(matches!(result, Err(Error::Canceled)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
