//! Downloads over one connection, resumed from an offset: what FTP and
//! SFTP share. The part file is the resume point; progress is published as
//! one segment, so the manager saves and restores it as for HTTP.

use std::io::{self, SeekFrom};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::{Duration, Instant};

use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::http::{self, Outcome, Progress, Segment, SegmentConfig};
use crate::{Error, Result};

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// For each read of a transfer (and each FTP reply).
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(30);
const BUFFER: usize = 64 * 1024;

/// One file being fetched: how much is written, and the retries so far.
pub(crate) struct Single<'a> {
    pub written: u64,
    pub total: Option<u64>,
    pub cancel: &'a CancellationToken,
    part_path: &'a Path,
    config: &'a SegmentConfig,
    progress: &'a watch::Sender<Progress>,
    failures: u32,
}

impl<'a> Single<'a> {
    /// Ready the part file: kept with `resume` (unless longer than the
    /// file), otherwise emptied.
    pub async fn start(
        part_path: &'a Path,
        final_path: &Path,
        total: Option<u64>,
        resume: bool,
        config: &'a SegmentConfig,
        progress: &'a watch::Sender<Progress>,
        cancel: &'a CancellationToken,
    ) -> Result<Self> {
        if fs::try_exists(final_path).await? {
            return Err(Error::DestinationExists(final_path.to_owned()));
        }
        if let Some(parent) = part_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut written = match fs::metadata(part_path).await {
            Ok(meta) if resume => meta.len(),
            _ => 0,
        };
        if total.is_some_and(|t| written > t) {
            written = 0;
        }
        if written == 0 {
            File::create(part_path).await?;
        } else {
            tracing::debug!(done = written, ?total, "resuming");
        }
        Ok(Self {
            written,
            total,
            cancel,
            part_path,
            config,
            progress,
            failures: 0,
        })
    }

    /// Whether the part file already holds the whole file.
    pub fn is_whole(&self) -> bool {
        self.written > 0 && self.total == Some(self.written)
    }

    /// Throw away what's there and start from byte 0.
    pub async fn restart(&mut self) -> Result<()> {
        self.written = 0;
        File::create(self.part_path).await?;
        Ok(())
    }

    /// Append what `reader` gives (it starts at `written`) until it ends:
    /// `true`, or `false` if the download was cancelled first. Read errors
    /// go through `read_error`.
    pub async fn copy<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
        read_error: fn(io::Error) -> Error,
    ) -> Result<bool> {
        self.publish(1);
        let mut file = OpenOptions::new().write(true).open(self.part_path).await?;
        // Anything past what was counted is a half-written leftover.
        file.set_len(self.written).await?;
        file.seek(SeekFrom::Start(self.written)).await?;
        let mut buffer = vec![0; BUFFER];
        let mut last_publish = Instant::now();
        let result = loop {
            let read = tokio::select! {
                _ = self.cancel.cancelled() => break Ok(false),
                read = tokio::time::timeout(READ_TIMEOUT, reader.read(&mut buffer)) => read,
            };
            let n = match read {
                Ok(Ok(0)) => break Ok(true),
                Ok(Ok(n)) => n,
                Ok(Err(e)) => break Err(read_error(e)),
                Err(_) => break Err(read_error(io::ErrorKind::TimedOut.into())),
            };
            if let Err(e) = file.write_all(&buffer[..n]).await {
                break Err(e.into());
            }
            self.written += n as u64;
            if last_publish.elapsed() >= self.config.progress_interval {
                self.publish(1);
                last_publish = Instant::now();
            }
            tokio::select! {
                _ = self.cancel.cancelled() => break Ok(false),
                _ = self.config.throttle.take(n as u64) => {}
            }
        };
        file.flush().await?;
        let ended = result?;
        if ended && self.total.is_some_and(|t| self.written < t) {
            return Err(Error::Incomplete);
        }
        Ok(ended)
    }

    /// After an attempt that started at `before` failed: `Ok(true)` to try
    /// again (after a backoff), `Ok(false)` if cancelled while waiting, or
    /// the error when it's not worth retrying. Only failures without
    /// progress in between count towards the limit.
    pub async fn retry(&mut self, error: Error, before: u64) -> Result<bool> {
        if self.written > before {
            self.failures = 0;
        }
        self.failures += 1;
        if !error.is_retryable() || self.failures > self.config.max_retries {
            self.publish(0);
            return Err(error);
        }
        let delay = http::backoff(self.config.retry_base_delay, self.failures);
        tracing::debug!(error = %error, attempt = self.failures, ?delay, "retrying transfer");
        tokio::select! {
            _ = self.cancel.cancelled() => Ok(false),
            _ = tokio::time::sleep(delay) => Ok(true),
        }
    }

    /// Wrap up: `Paused` keeps the part file; otherwise it's checked and
    /// moved to `final_path`.
    pub async fn end(self, completed: bool, final_path: &Path) -> Result<Outcome> {
        self.publish(0);
        if !completed {
            return Ok(Outcome::Paused);
        }
        let size = http::finish(self.part_path, final_path, self.total).await?;
        tracing::info!(path = %final_path.display(), size, "download complete");
        Ok(Outcome::Completed {
            path: final_path.to_owned(),
            size,
        })
    }

    fn publish(&self, connections: usize) {
        self.progress
            .send_replace(report(self.written, self.total, connections));
    }
}

/// The file as one segment, so resuming and progress work as for HTTP.
pub(crate) fn report(written: u64, total: Option<u64>, connections: usize) -> Progress {
    let end = match total {
        Some(total) if total > 0 => Some(total - 1),
        _ => written.checked_sub(1),
    };
    Progress {
        downloaded: written,
        total,
        connections,
        segments: end
            .map(|end| Segment {
                start: 0,
                end,
                written,
            })
            .into_iter()
            .collect(),
    }
}

/// The first of `host`'s addresses that answers (only those of the bound
/// adapter's family, when bound).
pub(crate) async fn tcp_to(host: &str, port: u16, local: Option<IpAddr>) -> io::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await?
        .filter(|a| local.is_none_or(|l| l.is_ipv4() == a.is_ipv4()))
        .collect();
    let mut last = io::Error::new(io::ErrorKind::NotFound, format!("no address for {host}"));
    for addr in addrs {
        match tcp(addr, local).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// A TCP connection to `addr`, sent from `local` when downloads are bound
/// to an adapter.
pub(crate) async fn tcp(addr: SocketAddr, local: Option<IpAddr>) -> io::Result<TcpStream> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    if let Some(ip) = local {
        socket.bind(SocketAddr::new(ip, 0))?;
    }
    tokio::time::timeout(CONNECT_TIMEOUT, socket.connect(addr))
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_one_segment() {
        let p = report(10, Some(100), 1);
        assert_eq!(
            p.segments,
            [Segment {
                start: 0,
                end: 99,
                written: 10
            }]
        );
        assert_eq!(report(0, None, 1).segments, []);
        assert_eq!(
            report(5, None, 0).segments,
            [Segment {
                start: 0,
                end: 4,
                written: 5
            }]
        );
    }
}
