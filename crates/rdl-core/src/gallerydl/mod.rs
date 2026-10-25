//! gallery-dl: resolving gallery pages (`-j`) and running downloads natively,
//! so its extractors, post-processors, rate limits and sign-in all work.
//!
//! Progress comes from what gallery-dl prints when stdout is not a terminal:
//! one path per finished file, `# ` before files it skipped because they
//! already exist (or are in the download archive). A gallery is downloaded
//! as a whole into its own folder; pausing kills the process, and running it
//! again skips the files already there.

mod options;
mod parse;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::process::{self, Line, Running};
use crate::request::{CookieFile, RequestContext};
use crate::{Error, Result};

pub use options::{
    FlagChoice, FlagDef, FlagGroup, FlagKind, FlagSchema, GalleryDlOptions, schema, split_args,
};
pub use parse::{Gallery, Resolved, ToolError, error_message, is_unsupported};

/// How long `-j` may take. Some sites page slowly through big galleries.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(90);

/// Resolving stops counting here; the download still gets everything.
const RESOLVE_LIMIT: u32 = 2000;

/// gallery-dl's exit status when no extractor matches the URL.
const NO_EXTRACTOR: i32 = 64;

/// Look up what `url` holds. An unsupported URL is an `Error::Tool` that
/// [`is_unsupported`] recognises. A gallery that takes too long to list
/// comes back `partial`, with whatever is known.
pub async fn resolve(
    exe: &Path,
    url: &Url,
    request: &RequestContext,
    cancel: &CancellationToken,
) -> Result<Resolved> {
    let cookies = CookieFile::create(request)?;
    let mut cmd = process::command(exe);
    cmd.arg("--config-ignore")
        .args(request_args(
            request,
            cookies.as_ref().map(CookieFile::path),
            &GalleryDlOptions::default(),
        ))
        .args(["-j", "--range"])
        .arg(format!("1-{RESOLVE_LIMIT}"))
        .arg("--")
        .arg(url.as_str());
    // Dropping the run on timeout kills it; `output`'s own limit stays out of the way.
    let run = process::output(cmd, RESOLVE_TIMEOUT * 4, cancel);
    let out = match tokio::time::timeout(RESOLVE_TIMEOUT, run).await {
        Ok(out) => out?,
        Err(_) => {
            tracing::info!(%url, "gallery-dl is slow to list this gallery; counting skipped");
            return Ok(Resolved {
                gallery: Gallery {
                    partial: true,
                    ..Gallery::default()
                },
                error: None,
            });
        }
    };
    let message = || {
        error_message(out.stderr.lines()).unwrap_or_else(|| format!("exited with {}", out.status))
    };
    if out.status.code() == Some(NO_EXTRACTOR) {
        let msg = message();
        return Err(Error::Tool(if is_unsupported(&msg) {
            msg
        } else {
            format!("Unsupported URL: {msg}")
        }));
    }
    if !out.stdout.trim_start().starts_with('[') {
        return Err(Error::Tool(message()));
    }
    let mut resolved = parse::resolved(&out.stdout, Some(RESOLVE_LIMIT))?;
    if resolved.error.is_none() && !out.status.success() {
        // An error that only made it to the log.
        resolved.error = Some(ToolError {
            kind: "Error".into(),
            message: message(),
        });
    }
    Ok(resolved)
}

/// The options the installed gallery-dl accepts, from its `--help`. Cached
/// per executable (each version has its own folder).
pub async fn supported_flags(exe: &Path) -> Result<Arc<HashSet<String>>> {
    static CACHE: LazyLock<Mutex<HashMap<PathBuf, Arc<HashSet<String>>>>> =
        LazyLock::new(Default::default);
    let cache = || CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(flags) = cache().get(exe) {
        return Ok(Arc::clone(flags));
    }
    let mut cmd = process::command(exe);
    cmd.arg("--help");
    // A never-run PyInstaller exe can sit in an antivirus scan for a while.
    let out = process::output(cmd, Duration::from_secs(300), &CancellationToken::new()).await?;
    let flags = parse::help_flags(&out.stdout);
    if flags.is_empty() {
        return Err(Error::Tool(format!(
            "gallery-dl --help listed no options: {}",
            out.error_message()
        )));
    }
    let flags = Arc::new(flags);
    cache().insert(exe.to_owned(), Arc::clone(&flags));
    Ok(flags)
}

/// The browser's user agent and cookies (a `cookies.txt` at `cookies`) as
/// gallery-dl options, unless `options` already pick their own. The referer
/// isn't passed: extractors set the one their site expects.
fn request_args(
    request: &RequestContext,
    cookies: Option<&Path>,
    options: &GalleryDlOptions,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(ua) = &request.user_agent
        && !options.is_set("user-agent")
        && !options.is_set("browser-user-agent")
    {
        args.extend(["--user-agent".into(), ua.clone()]);
    }
    if let Some(path) = cookies
        && !options.is_set("cookies")
        && !options.is_set("cookies-from-browser")
    {
        args.extend(["--cookies".into(), path.to_string_lossy().into_owned()]);
    }
    args
}

/// What to download and where.
#[derive(Debug, Clone)]
pub struct RunSpec {
    pub exe: PathBuf,
    /// Folder holding ffmpeg.exe, for ugoira conversion; put on `PATH`.
    pub ffmpeg_dir: Option<PathBuf>,
    pub url: Url,
    /// Exact folder for the files (`-D`): no sub-folders.
    pub dir: PathBuf,
    /// Download archive (SQLite) to record and skip fetched files.
    pub archive: Option<PathBuf>,
    pub options: GalleryDlOptions,
    /// Flags the installed version knows; others are dropped.
    pub available: Option<Arc<HashSet<String>>>,
    /// User agent and cookies from the browser.
    pub request: RequestContext,
    /// The cookies written out for this run; [`download`] fills it in.
    pub cookie_file: Option<PathBuf>,
}

impl RunSpec {
    pub fn args(&self) -> Result<Vec<String>> {
        let mut args: Vec<String> = Vec::new();
        if !self.options.user_config {
            args.push("--config-ignore".into());
        }
        args.push("-D".into());
        args.push(self.dir.to_string_lossy().into_owned());
        if let (true, Some(archive)) = (self.options.archive, &self.archive) {
            args.push("--download-archive".into());
            args.push(archive.to_string_lossy().into_owned());
        }
        args.extend(
            self.options
                .flag_args(schema(), self.available.as_deref())?,
        );
        args.extend(request_args(
            &self.request,
            self.cookie_file.as_deref(),
            &self.options,
        ));
        args.push("--".into());
        args.push(self.url.to_string());
        Ok(args)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    /// A file is done: downloaded now, or skipped because it was there before.
    File {
        path: PathBuf,
        size: u64,
        skipped: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Completed {
        files: u32,
        bytes: u64,
    },
    /// Cancelled; running again skips the files already done.
    Paused,
}

/// Run a download. Events arrive on `events` while it runs.
pub async fn download(
    spec: &RunSpec,
    events: &mpsc::UnboundedSender<RunEvent>,
    cancel: &CancellationToken,
) -> Result<RunOutcome> {
    let cookies = CookieFile::create(&spec.request)?;
    let spec = &RunSpec {
        dir: std::path::absolute(&spec.dir)?,
        cookie_file: cookies.as_ref().map(|c| c.path().to_owned()),
        ..spec.clone()
    };
    tokio::fs::create_dir_all(&spec.dir).await?;
    if let Some(parent) = spec.archive.as_deref().and_then(Path::parent) {
        tokio::fs::create_dir_all(parent).await?;
    }
    let args = spec.args()?;
    let mut cmd = process::command(&spec.exe);
    cmd.args(&args).current_dir(&spec.dir);
    if let Some(dir) = &spec.ffmpeg_dir {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined =
            std::env::join_paths(std::iter::once(dir.clone()).chain(std::env::split_paths(&path)))
                .map_err(|e| Error::Tool(e.to_string()))?;
        cmd.env("PATH", joined);
    }
    tracing::debug!(?args, "starting gallery-dl");
    let mut running = Running::spawn(cmd)?;

    let (mut files, mut bytes) = (0u32, 0u64);
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
                if let Some((path, skipped)) = file_line(&text) {
                    let size = tokio::fs::metadata(&path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);
                    files += 1;
                    bytes += size;
                    let _ = events.send(RunEvent::File {
                        path,
                        size,
                        skipped,
                    });
                } else {
                    tracing::trace!(line = %text, "gallery-dl stdout");
                }
            }
            Line::Err(text) => {
                tracing::trace!(line = %text, "gallery-dl");
                stderr_tail.push(text);
                if stderr_tail.len() > 50 {
                    stderr_tail.remove(0);
                }
            }
        }
    }
    let status = running.wait().await?;
    if !status.success() {
        let message = error_message(stderr_tail.iter().map(String::as_str))
            .unwrap_or_else(|| format!("gallery-dl exited with {status}"));
        return Err(Error::Tool(if files > 0 {
            format!("{message} (after {files} files; retrying skips those)")
        } else {
            message
        }));
    }
    Ok(RunOutcome::Completed { files, bytes })
}

/// A finished file on stdout: an absolute path, `# ` first when skipped.
fn file_line(text: &str) -> Option<(PathBuf, bool)> {
    let (skipped, rest) = match text.strip_prefix("# ") {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let path = Path::new(rest.trim_end());
    path.is_absolute().then(|| (path.to_owned(), skipped))
}

/// gallery-dl's unfinished files in a gallery folder.
pub fn partial_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "part"))
        .collect()
}

/// The download archive for a site: one per host, so galleries from the same
/// site share what was fetched before.
pub fn archive_path(archives_dir: &Path, url: &Url) -> PathBuf {
    let host = url.host_str().unwrap_or("other");
    let host = host.strip_prefix("www.").unwrap_or(host);
    archives_dir
        .join("gallery-dl")
        .join(format!("{}.sqlite3", crate::http::filename::sanitize(host)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn reads_file_lines() {
        assert_eq!(
            file_line(r"C:\dl\Trip\001.jpg"),
            Some((PathBuf::from(r"C:\dl\Trip\001.jpg"), false))
        );
        assert_eq!(
            file_line(r"# C:\dl\Trip\002.jpg"),
            Some((PathBuf::from(r"C:\dl\Trip\002.jpg"), true))
        );
        assert_eq!(file_line("some progress text"), None);
        assert_eq!(file_line("# relative.jpg"), None);
    }

    #[test]
    fn args_pin_the_folder_and_archive() {
        let mut spec = RunSpec {
            exe: "gallery-dl.exe".into(),
            ffmpeg_dir: None,
            url: Url::parse("https://imgur.com/a/abc").unwrap(),
            dir: r"C:\dl\Trip".into(),
            archive: Some(r"C:\data\archives\gallery-dl\imgur.com.sqlite3".into()),
            options: GalleryDlOptions::default(),
            available: None,
            request: RequestContext::default(),
            cookie_file: None,
        };
        let args = spec.args().unwrap();
        assert!(!args.iter().any(|a| a == "--cookies" || a == "--user-agent"));
        let after = |args: &[String], flag: &str| {
            let i = args.iter().position(|a| a == flag).unwrap();
            args[i + 1].clone()
        };
        assert_eq!(args[0], "--config-ignore");
        assert_eq!(after(&args, "-D"), r"C:\dl\Trip");
        assert!(after(&args, "--download-archive").ends_with("imgur.com.sqlite3"));
        assert_eq!(args[args.len() - 2..], ["--", "https://imgur.com/a/abc"]);

        spec.options.archive = false;
        spec.options.user_config = true;
        let args = spec.args().unwrap();
        assert!(
            !args
                .iter()
                .any(|a| a == "--download-archive" || a == "--config-ignore")
        );
    }

    #[test]
    fn browser_cookies_and_agent_yield_to_chosen_options() {
        let request = RequestContext {
            referer: Some("https://example.com/page".into()),
            user_agent: Some("TestBrowser/1".into()),
            cookies: Vec::new(),
        };
        let cookies = Path::new(r"C:\tmp\c.txt");
        let mut options = GalleryDlOptions::default();
        assert_eq!(
            request_args(&request, Some(cookies), &options),
            [
                "--user-agent",
                "TestBrowser/1",
                "--cookies",
                r"C:\tmp\c.txt"
            ]
        );
        options
            .flags
            .insert("cookies".into(), r"D:\mine.txt".into());
        options
            .flags
            .insert("browser-user-agent".into(), "true".into());
        assert!(request_args(&request, Some(cookies), &options).is_empty());
        // Switched off or blank counts as not chosen.
        options.flags.insert("cookies".into(), " ".into());
        options
            .flags
            .insert("browser-user-agent".into(), "false".into());
        assert_eq!(request_args(&request, Some(cookies), &options).len(), 4);
    }

    #[test]
    fn archives_are_per_host() {
        let dir = Path::new("data");
        let a = archive_path(dir, &Url::parse("https://www.imgur.com/a/x").unwrap());
        let b = archive_path(dir, &Url::parse("https://imgur.com/gallery/y").unwrap());
        assert_eq!(a, b);
        assert_eq!(a, Path::new("data/gallery-dl/imgur.com.sqlite3"));
    }

    #[test]
    fn finds_part_files() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["001.jpg", "002.jpg.part", "notes.txt"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let found = partial_files(dir.path());
        assert_eq!(found, [dir.path().join("002.jpg.part")]);
    }
}
