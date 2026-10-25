//! yt-dlp: resolving pages to media info (`-J`) and running downloads with
//! machine-readable progress.
//!
//! Output from a download is tagged so it can't be confused with yt-dlp's own
//! messages: `RDLINFO` (planned file and size, once per item), `RDLPROG`
//! (progress JSON, one line per update thanks to `--newline`) and `RDLDONE`
//! (the final path after merging and moving).

mod options;
mod parse;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::process::{self, Line, Running};
use crate::request::{CookieFile, RequestContext};
use crate::{Error, Result};

pub use options::{FormatPreset, YtDlpOptions};
pub use parse::{FormatInfo, Media, MediaInfo, PlaylistEntry};

/// How long `-J` may take. Big playlists are slow even when flat.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(120);

/// Default output name: title plus id keeps re-uploads with the same title apart.
const DEFAULT_TEMPLATE: &str = "%(title).180B [%(id)s].%(ext)s";

/// Arguments every run gets: ignore the user's own yt-dlp config so behaviour
/// is predictable, never treat a video URL as its playlist, no colours, and
/// UTF-8 output (yt-dlp otherwise follows the console code page).
fn base_args() -> [&'static str; 6] {
    [
        "--ignore-config",
        "--no-playlist",
        "--no-colors",
        "--no-warnings",
        "--encoding",
        "utf-8",
    ]
}

/// yt-dlp reports every chunk; the UI needs a few updates a second.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Look up what `url` points to. A playlist comes back flat: its entries have
/// titles and URLs but no formats.
pub async fn resolve(
    exe: &Path,
    url: &Url,
    request: &RequestContext,
    cancel: &CancellationToken,
) -> Result<Media> {
    let cookies = CookieFile::create(request)?;
    let mut cmd = process::command(exe);
    cmd.args(base_args())
        .args(request_args(
            request,
            cookies.as_ref().map(CookieFile::path),
        ))
        .args(["-J", "--flat-playlist", "--"])
        .arg(url.as_str());
    let out = process::output(cmd, RESOLVE_TIMEOUT, cancel).await?;
    if !out.status.success() {
        return Err(Error::Tool(out.error_message()));
    }
    parse::media(&out.stdout)
}

/// Referer, user agent and cookies (a `cookies.txt` at `cookies`) as yt-dlp
/// options.
fn request_args(request: &RequestContext, cookies: Option<&Path>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(referer) = &request.referer {
        args.extend(["--add-header".into(), format!("Referer:{referer}")]);
    }
    if let Some(ua) = &request.user_agent {
        args.extend(["--add-header".into(), format!("User-Agent:{ua}")]);
    }
    if let Some(path) = cookies {
        args.extend(["--cookies".into(), path.to_string_lossy().into_owned()]);
    }
    args
}

/// What to download and where.
#[derive(Debug, Clone)]
pub struct RunSpec {
    pub exe: PathBuf,
    /// Folder holding ffmpeg.exe; without it yt-dlp picks single-file formats.
    pub ffmpeg_dir: Option<PathBuf>,
    pub url: Url,
    pub dest_dir: PathBuf,
    /// Output name without extension; `None` uses title and id.
    pub name: Option<String>,
    pub options: YtDlpOptions,
    /// Referer, user agent and cookies from the browser.
    pub request: RequestContext,
    /// The cookies written out for this run; [`download`] fills it in.
    pub cookie_file: Option<PathBuf>,
    /// Bytes per second (`--limit-rate`); fixed for the run.
    pub limit_rate: Option<u64>,
}

impl RunSpec {
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = base_args().iter().map(|s| s.to_string()).collect();
        let template = match &self.name {
            Some(name) => format!(
                "{}.%(ext)s",
                options::strip_media_ext(name).replace('%', "%%")
            ),
            None => DEFAULT_TEMPLATE.to_owned(),
        };
        args.extend(
            [
                "--newline",
                "--progress",
                "--no-mtime",
                "--continue",
                "--no-simulate",
                "--progress-template",
                "download:RDLPROG %(progress.{status,downloaded_bytes,total_bytes,total_bytes_estimate,speed,filename})j",
                "--print",
                "before_dl:RDLINFO %(.{filename,filesize,filesize_approx})j",
                "--print",
                "after_move:RDLDONE %(filepath)j",
                "-P",
            ]
            .map(str::to_owned),
        );
        args.push(self.dest_dir.to_string_lossy().into_owned());
        args.push("-o".into());
        args.push(template);
        if let Some(dir) = &self.ffmpeg_dir {
            args.push("--ffmpeg-location".into());
            args.push(dir.to_string_lossy().into_owned());
        }
        args.extend(self.options.format_args());
        args.extend(request_args(&self.request, self.cookie_file.as_deref()));
        if let Some(rate) = self.limit_rate {
            args.extend(["--limit-rate".into(), rate.to_string()]);
        }
        args.push("--".into());
        args.push(self.url.to_string());
        args
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunEvent {
    /// Where the file will end up and roughly how big it is.
    Planned { path: PathBuf, size: Option<u64> },
    Progress {
        downloaded: u64,
        total: Option<u64>,
        speed_bps: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Completed {
        path: PathBuf,
        size: u64,
    },
    /// Cancelled; running again with the same spec continues from the `.part` files.
    Paused,
}

/// Run a download. Events arrive on `events` while it runs.
pub async fn download(
    spec: &RunSpec,
    events: &mpsc::UnboundedSender<RunEvent>,
    cancel: &CancellationToken,
) -> Result<RunOutcome> {
    // yt-dlp reports paths relative to its working folder, so a relative
    // destination (even a drive-relative `C:dir`) would be resolved twice.
    let cookies = CookieFile::create(&spec.request)?;
    let spec = &RunSpec {
        dest_dir: std::path::absolute(&spec.dest_dir)?,
        cookie_file: cookies.as_ref().map(|c| c.path().to_owned()),
        ..spec.clone()
    };
    tokio::fs::create_dir_all(&spec.dest_dir).await?;
    let mut cmd = process::command(&spec.exe);
    cmd.args(spec.args()).current_dir(&spec.dest_dir);
    tracing::debug!(args = ?spec.args(), "starting yt-dlp");
    let mut running = Running::spawn(cmd)?;

    let mut tracker = Tracker::default();
    let mut last_progress: Option<std::time::Instant> = None;
    let mut final_path = None;
    let mut stderr_tail: Vec<String> = Vec::new();
    loop {
        let line = tokio::select! {
            _ = cancel.cancelled() => {
                running.kill().await;
                return Ok(RunOutcome::Paused);
            }
            line = running.next_line() => line,
        };
        let Some(line) = line else { break };
        match line {
            Line::Out(text) => {
                if let Some(json) = text.strip_prefix("RDLPROG ") {
                    if let Some(p) = parse::progress(json) {
                        let (downloaded, total) = tracker.update(&p);
                        let due = last_progress.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL);
                        if !due && p.status != "finished" {
                            continue;
                        }
                        last_progress = Some(std::time::Instant::now());
                        let _ = events.send(RunEvent::Progress {
                            downloaded,
                            total,
                            speed_bps: p.speed.map(|s| s as u64),
                        });
                    }
                } else if let Some(json) = text.strip_prefix("RDLINFO ") {
                    if let Some(info) = parse::planned(json) {
                        let path = absolute(&spec.dest_dir, &info.filename);
                        tracker.planned_size = info.size;
                        let _ = events.send(RunEvent::Planned {
                            path,
                            size: info.size,
                        });
                    }
                } else if let Some(json) = text.strip_prefix("RDLDONE ")
                    && let Ok(path) = serde_json::from_str::<String>(json)
                {
                    final_path = Some(absolute(&spec.dest_dir, &path));
                }
            }
            Line::Err(text) => {
                tracing::trace!(line = %text, "yt-dlp");
                stderr_tail.push(text);
                if stderr_tail.len() > 50 {
                    stderr_tail.remove(0);
                }
            }
        }
    }
    let status = running.wait().await?;
    if !status.success() {
        let message = process::error_message(stderr_tail.iter().map(String::as_str))
            .unwrap_or_else(|| format!("yt-dlp exited with {status}"));
        return Err(Error::Tool(message));
    }
    let path =
        final_path.ok_or_else(|| Error::Tool("yt-dlp finished without reporting a file".into()))?;
    let size = tokio::fs::metadata(&path).await?.len();
    Ok(RunOutcome::Completed { path, size })
}

fn absolute(dir: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_owned()
    } else {
        dir.join(path)
    }
}

/// Sums progress across the separate files of one item (video and audio
/// streams are downloaded one after the other, then merged).
#[derive(Default)]
struct Tracker {
    files: HashMap<String, (u64, Option<u64>)>,
    planned_size: Option<u64>,
}

impl Tracker {
    fn update(&mut self, p: &parse::Progress) -> (u64, Option<u64>) {
        let total = p.total_bytes.or(p.total_bytes_estimate.map(|t| t as u64));
        let downloaded = match (p.status.as_str(), total) {
            ("finished", Some(t)) => t.max(p.downloaded_bytes.unwrap_or(0)),
            _ => p.downloaded_bytes.unwrap_or(0),
        };
        let key = p.filename.clone().unwrap_or_default();
        self.files.insert(key, (downloaded, total));

        let done: u64 = self.files.values().map(|(d, _)| d).sum();
        let known: Option<u64> = self.files.values().map(|(_, t)| *t).sum();
        // yt-dlp's estimate covers all streams; per-file totals only those started.
        let total = match (self.planned_size, known) {
            (Some(planned), Some(k)) => Some(planned.max(k)),
            (planned, k) => planned.or(k),
        };
        (done, total.map(|t| t.max(done)))
    }
}

/// yt-dlp's leftovers for an unfinished download planned as `planned`:
/// `.part` files, fragment files, `.ytdl` state and unmerged `.fNNN` streams.
pub fn partial_files(planned: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (
        planned.parent(),
        planned
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned()),
    ) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.");
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|path| {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                return false;
            };
            let Some(rest) = name.strip_prefix(&prefix) else {
                return false;
            };
            rest.ends_with(".part")
                || rest.ends_with(".ytdl")
                || rest.contains(".part-Frag")
                || is_format_stream(rest)
        })
        .collect()
}

/// `f137.mp4`: a single stream waiting to be merged.
fn is_format_stream(rest: &str) -> bool {
    rest.strip_prefix('f')
        .and_then(|r| r.split_once('.'))
        .is_some_and(|(id, _)| {
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(status: &str, file: &str, done: u64, total: Option<u64>) -> parse::Progress {
        parse::Progress {
            status: status.into(),
            downloaded_bytes: Some(done),
            total_bytes: total,
            total_bytes_estimate: None,
            speed: None,
            filename: Some(file.into()),
        }
    }

    #[test]
    fn tracker_sums_streams() {
        let mut t = Tracker {
            planned_size: Some(1000),
            ..Default::default()
        };
        assert_eq!(
            t.update(&progress("downloading", "v", 100, Some(800))),
            (100, Some(1000))
        );
        assert_eq!(
            t.update(&progress("finished", "v", 800, Some(800))),
            (800, Some(1000))
        );
        assert_eq!(
            t.update(&progress("downloading", "a", 150, Some(250))),
            (950, Some(1050))
        );
    }

    #[test]
    fn tracker_without_estimate_uses_known_totals() {
        let mut t = Tracker::default();
        assert_eq!(
            t.update(&progress("downloading", "v", 10, None)),
            (10, None)
        );
        assert_eq!(
            t.update(&progress("downloading", "v", 20, Some(40))),
            (20, Some(40))
        );
    }

    #[test]
    fn args_name_the_output_and_pass_ffmpeg() {
        let spec = RunSpec {
            exe: "yt-dlp.exe".into(),
            ffmpeg_dir: Some(r"C:\tools\ffmpeg".into()),
            url: Url::parse("https://example.com/watch?v=1").unwrap(),
            dest_dir: r"C:\dl".into(),
            name: Some("100% Fun.mp4".into()),
            options: YtDlpOptions::default(),
            request: RequestContext::default(),
            cookie_file: None,
            limit_rate: None,
        };
        let args = spec.args();
        assert!(!args.iter().any(|a| a == "--cookies" || a == "--add-header"));
        let after = |flag: &str| {
            let i = args.iter().position(|a| a == flag).unwrap();
            args[i + 1].clone()
        };
        assert_eq!(after("-o"), "100%% Fun.%(ext)s");
        assert_eq!(after("-P"), r"C:\dl");
        assert_eq!(after("--ffmpeg-location"), r"C:\tools\ffmpeg");
        assert_eq!(args.last().unwrap(), "https://example.com/watch?v=1");
        assert_eq!(args[args.len() - 2], "--");
    }

    #[test]
    fn args_pass_the_browser_request() {
        let spec = RunSpec {
            exe: "yt-dlp.exe".into(),
            ffmpeg_dir: None,
            url: Url::parse("https://example.com/watch?v=1").unwrap(),
            dest_dir: r"C:\dl".into(),
            name: None,
            options: YtDlpOptions::default(),
            request: RequestContext {
                referer: Some("https://example.com/page".into()),
                user_agent: Some("TestBrowser/1".into()),
                cookies: Vec::new(),
            },
            cookie_file: Some(r"C:\tmp\c.txt".into()),
            limit_rate: Some(1_048_576),
        };
        let args = spec.args();
        let joined = args.join(" ");
        assert!(joined.contains("--add-header Referer:https://example.com/page"));
        assert!(joined.contains("--add-header User-Agent:TestBrowser/1"));
        assert!(joined.contains(r"--cookies C:\tmp\c.txt"));
        assert!(joined.contains("--limit-rate 1048576"));
        assert_eq!(args[args.len() - 2], "--");
    }

    #[test]
    fn finds_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "Clip [abc].f137.mp4.part",
            "Clip [abc].f140.m4a",
            "Clip [abc].mp4.ytdl",
            "Clip [abc].f137.mp4.part-Frag3",
            "Clip [abc].mp4",
            "Clip [abcd].f1.mp4.part",
            "Other.mp4.part",
        ] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let mut found: Vec<String> = partial_files(&dir.path().join("Clip [abc].mp4"))
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                "Clip [abc].f137.mp4.part",
                "Clip [abc].f137.mp4.part-Frag3",
                "Clip [abc].f140.m4a",
                "Clip [abc].mp4.ytdl",
            ]
        );
    }
}
