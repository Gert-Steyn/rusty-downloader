//! FTP and FTPS downloads: one connection per file, resumed with `REST`.
//!
//! `ftp://` is plain FTP. `ftps://` is explicit FTPS: `AUTH TLS` on the
//! control connection, then private data connections, with the server's
//! certificate checked against the Windows store as for HTTPS. Implicit FTPS
//! (TLS from the first byte, port 990) isn't supported.
//!
//! The login comes from the link (`ftp://user:pass@host/…`), else from the
//! host's account, else it's anonymous. Proxies don't apply; the adapter
//! binding does, for the control and the data connections alike. Paths are
//! relative to the login's home folder, as RFC 1738 has it (`%2F` at the
//! start makes one absolute).

use std::io::{self, SeekFrom};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use percent_encoding::percent_decode_str;
use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt;
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Mode};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use crate::accounts::Login;
use crate::http::{self, Outcome, ProbeResult, Progress, Segment, SegmentConfig, filename};
use crate::{Error, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// For a command's reply and for each read of a transfer.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const BUFFER: usize = 64 * 1024;

/// Whether `url` is an FTP or FTPS link.
pub fn is_ftp(url: &Url) -> bool {
    matches!(url.scheme(), "ftp" | "ftps")
}

/// Where a file is and how to log in for it.
#[derive(Clone)]
pub struct Location {
    url: Url,
    host: String,
    port: u16,
    secure: bool,
    path: String,
    username: String,
    password: String,
    /// The bound adapter's address, if downloads are bound to one.
    local: Option<IpAddr>,
}

impl std::fmt::Debug for Location {
    // Never the password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Location")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("secure", &self.secure)
            .field("path", &self.path)
            .field("username", &self.username)
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

impl Location {
    /// `login` is the host's account, used when the link has no login of its own.
    pub fn new(url: &Url, login: Option<Login>, local: Option<IpAddr>) -> Result<Self> {
        let invalid = |why: &str| Error::InvalidUrl(format!("{url}: {why}"));
        let secure = match url.scheme() {
            "ftp" => false,
            "ftps" => true,
            _ => return Err(invalid("not an FTP link")),
        };
        let host = match url.host() {
            Some(Host::Domain(name)) => name.to_owned(),
            Some(Host::Ipv4(ip)) => ip.to_string(),
            Some(Host::Ipv6(ip)) => ip.to_string(),
            None => return Err(invalid("no host")),
        };
        let port = url.port().unwrap_or(21);
        if secure && port == 990 {
            return Err(Error::Unsupported(
                "implicit FTPS (port 990); explicit FTPS works on the usual port".into(),
            ));
        }
        let path = decode(url.path().strip_prefix('/').unwrap_or(url.path()));
        if path.is_empty() || path.ends_with('/') {
            return Err(invalid("a folder, not a file"));
        }
        let (username, password) = if !url.username().is_empty() {
            (
                decode(url.username()),
                url.password().map(decode).unwrap_or_default(),
            )
        } else if let Some(login) = login.filter(|l| l.is_for(&host)) {
            (login.username, login.password)
        } else {
            ("anonymous".to_owned(), "anonymous@".to_owned())
        };
        Ok(Self {
            url: url.clone(),
            host,
            port,
            secure,
            path,
            username,
            password,
            local,
        })
    }

    /// The file's name, from the last part of its path.
    pub fn filename(&self) -> String {
        let name = self.path.rsplit('/').next().unwrap_or_default();
        filename::sanitize(name)
    }
}

fn decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// Log in and ask for the file's size and modification time. `SIZE`
/// refusing with `550` means there's no such file (or it's a folder);
/// a server without `SIZE` leaves the size unknown.
pub async fn probe(location: &Location) -> Result<ProbeResult> {
    let mut ftp = connect(location).await?;
    let size = match within(ftp.size(&location.path)).await {
        Ok(size) => Some(size as u64),
        Err(Error::FtpReply { code: 550, message }) => {
            return Err(Error::FtpReply { code: 550, message });
        }
        Err(e @ Error::FtpReply { .. }) => {
            tracing::debug!(error = %e, "server has no SIZE");
            None
        }
        Err(e) => return Err(e),
    };
    let modified = within(ftp.mdtm(&location.path))
        .await
        .ok()
        .map(|t| t.format("%Y%m%d%H%M%S").to_string());
    let _ = within(ftp.quit()).await;
    let result = ProbeResult {
        final_url: location.url.clone(),
        size,
        accepts_ranges: true,
        etag: None,
        last_modified: modified,
        content_type: None,
        filename: location.filename(),
    };
    tracing::debug!(?location, ?result, "probed");
    Ok(result)
}

/// Download `location` (already probed) into `part_path`, then move it to
/// `final_path`. With `resume`, carries on from what the part file holds.
#[allow(clippy::too_many_arguments)]
pub async fn download(
    location: &Location,
    probe: &ProbeResult,
    part_path: &Path,
    final_path: &Path,
    resume: bool,
    config: &SegmentConfig,
    progress: &watch::Sender<Progress>,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    if fs::try_exists(final_path).await? {
        return Err(Error::DestinationExists(final_path.to_owned()));
    }
    if let Some(parent) = part_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let total = probe.size;
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

    let mut failures = 0;
    let outcome = loop {
        let before = written;
        let error = match transfer(
            location,
            total,
            part_path,
            &mut written,
            config,
            progress,
            cancel,
        )
        .await
        {
            Ok(outcome) => break outcome,
            Err(e) => e,
        };
        if written > before {
            failures = 0;
        }
        // A server that can't resume refuses `REST` as an unknown command.
        if before > 0
            && written == before
            && matches!(
                error,
                Error::FtpReply {
                    code: 500..=504,
                    ..
                }
            )
        {
            tracing::warn!(error = %error, "server can't resume; starting over");
            written = 0;
            File::create(part_path).await?;
            continue;
        }
        failures += 1;
        if !error.is_retryable() || failures > config.max_retries {
            progress.send_replace(report(written, total, 0));
            return Err(error);
        }
        let delay = http::backoff(config.retry_base_delay, failures);
        tracing::debug!(error = %error, attempt = failures, ?delay, "retrying FTP transfer");
        tokio::select! {
            _ = cancel.cancelled() => break Outcome::Paused,
            _ = tokio::time::sleep(delay) => {}
        }
    };
    progress.send_replace(report(written, total, 0));
    if outcome == Outcome::Paused {
        return Ok(outcome);
    }
    let size = http::finish(part_path, final_path, total).await?;
    tracing::info!(path = %final_path.display(), size, "download complete");
    Ok(Outcome::Completed {
        path: final_path.to_owned(),
        size,
    })
}

/// One connection's worth: `REST` to where the part file ends, `RETR`, and
/// append until the server is done or the download is cancelled.
async fn transfer(
    location: &Location,
    total: Option<u64>,
    part_path: &Path,
    written: &mut u64,
    config: &SegmentConfig,
    progress: &watch::Sender<Progress>,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let mut ftp = tokio::select! {
        _ = cancel.cancelled() => return Ok(Outcome::Paused),
        ftp = connect(location) => ftp?,
    };
    if total == Some(*written) && *written > 0 {
        let _ = within(ftp.quit()).await;
        return Ok(Outcome::Completed {
            path: PathBuf::new(),
            size: *written,
        });
    }
    if *written > 0 {
        ftp.resume_transfer(*written as usize)
            .await
            .map_err(Error::from)?;
    }
    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Ok(Outcome::Paused),
        stream = within(ftp.retr_as_stream(&location.path)) => stream?,
    };
    progress.send_replace(report(*written, total, 1));

    let mut file = OpenOptions::new().write(true).open(part_path).await?;
    // Anything past what was counted is a half-written leftover.
    file.set_len(*written).await?;
    file.seek(SeekFrom::Start(*written)).await?;
    let mut buffer = vec![0; BUFFER];
    let mut last_publish = Instant::now();
    let result = loop {
        let read = tokio::select! {
            _ = cancel.cancelled() => break Ok(false),
            read = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buffer)) => read,
        };
        let n = match read {
            Ok(Ok(0)) => break Ok(true),
            Ok(Ok(n)) => n,
            Ok(Err(e)) => break Err(Error::FtpConnection(e)),
            Err(_) => break Err(Error::FtpConnection(io::ErrorKind::TimedOut.into())),
        };
        if let Err(e) = file.write_all(&buffer[..n]).await {
            break Err(e.into());
        }
        *written += n as u64;
        if last_publish.elapsed() >= config.progress_interval {
            progress.send_replace(report(*written, total, 1));
            last_publish = Instant::now();
        }
        tokio::select! {
            _ = cancel.cancelled() => break Ok(false),
            _ = config.throttle.take(n as u64) => {}
        }
    };
    file.flush().await?;
    if !result? {
        return Ok(Outcome::Paused); // dropping the connection ends the transfer
    }
    within(stream.finish()).await?;
    let _ = within(ftp.quit()).await;
    if total.is_some_and(|t| *written < t) {
        return Err(Error::Incomplete);
    }
    Ok(Outcome::Completed {
        path: PathBuf::new(),
        size: *written,
    })
}

/// The file as one segment, so resuming and progress work as for HTTP.
fn report(written: u64, total: Option<u64>, connections: usize) -> Progress {
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

/// Connect, switch to TLS for `ftps://`, log in, and set binary mode.
async fn connect(location: &Location) -> Result<AsyncRustlsFtpStream> {
    let local = location.local;
    let control = tcp_to(&location.host, location.port, local).await?;
    let ipv6 = control.peer_addr().map_err(Error::FtpConnection)?.is_ipv6();
    let mut ftp = within(AsyncRustlsFtpStream::connect_with_stream(control))
        .await?
        .passive_stream_builder(move |addr| {
            Box::pin(async move { tcp(addr, local).await.map_err(FtpError::ConnectionError) })
        });
    // Data connections go where the control connection went: a server
    // behind NAT often names its private address in `PASV`.
    ftp.set_passive_nat_workaround(true);
    if ipv6 {
        ftp.set_mode(Mode::ExtendedPassive);
    }
    if location.secure {
        ftp = within(ftp.into_secure(tls()?, &location.host)).await?;
    }
    within(ftp.login(&location.username, &location.password)).await?;
    within(ftp.transfer_type(FileType::Binary)).await?;
    Ok(ftp)
}

/// The first of `host`'s addresses that answers (only those of the bound
/// adapter's family, when bound).
async fn tcp_to(host: &str, port: u16, local: Option<IpAddr>) -> Result<TcpStream> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(Error::FtpConnection)?
        .filter(|a| local.is_none_or(|l| l.is_ipv4() == a.is_ipv4()))
        .collect();
    let mut last = io::Error::new(io::ErrorKind::NotFound, format!("no address for {host}"));
    for addr in addrs {
        match tcp(addr, local).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last = e,
        }
    }
    Err(Error::FtpConnection(last))
}

async fn tcp(addr: SocketAddr, local: Option<IpAddr>) -> io::Result<TcpStream> {
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

/// TLS with the Windows certificate store. The data connections reuse the
/// control connection's session, which many servers insist on.
fn tls() -> Result<AsyncRustlsConnector> {
    let setup = |e: rustls::Error| Error::Ftp(format!("TLS setup failed: {e}"));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(setup)?
        .with_platform_verifier()
        .map_err(setup)?
        .with_no_client_auth();
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)).into())
}

/// A command, given [`READ_TIMEOUT`] to answer.
async fn within<T>(command: impl Future<Output = suppaftp::FtpResult<T>>) -> Result<T> {
    match tokio::time::timeout(READ_TIMEOUT, command).await {
        Ok(result) => result.map_err(Error::from),
        Err(_) => Err(Error::FtpConnection(io::ErrorKind::TimedOut.into())),
    }
}

impl From<FtpError> for Error {
    fn from(e: FtpError) -> Self {
        match e {
            FtpError::ConnectionError(e) => Error::FtpConnection(e),
            FtpError::UnexpectedResponse(response) => Error::FtpReply {
                code: response.status.code(),
                message: String::from_utf8_lossy(&response.body)
                    .trim()
                    .trim_start_matches(|c: char| c.is_ascii_digit() || c == '-' || c == ' ')
                    .to_owned(),
            },
            FtpError::SecureError(e) => Error::Ftp(format!("TLS: {e}")),
            other => Error::Ftp(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(url: &str, login: Option<Login>) -> Result<Location> {
        Location::new(&Url::parse(url).unwrap(), login, None)
    }

    fn login(site: &str) -> Login {
        Login {
            site: site.into(),
            username: "acct".into(),
            password: "pw".into(),
        }
    }

    #[test]
    fn locations_from_links() {
        let l = location("ftp://ftp.example.com/pub/a%20b.iso", None).unwrap();
        assert_eq!(
            (l.host.as_str(), l.port, l.secure),
            ("ftp.example.com", 21, false)
        );
        assert_eq!(l.path, "pub/a b.iso");
        assert_eq!(l.filename(), "a b.iso");
        assert_eq!(
            (l.username.as_str(), l.password.as_str()),
            ("anonymous", "anonymous@")
        );

        let l = location("ftps://me:p%40ss@[::1]:2121/%2Fsrv/f.bin", None).unwrap();
        assert_eq!((l.host.as_str(), l.port, l.secure), ("::1", 2121, true));
        assert_eq!(l.path, "/srv/f.bin");
        assert_eq!((l.username.as_str(), l.password.as_str()), ("me", "p@ss"));
        assert!(!format!("{l:?}").contains("p@ss"));

        assert!(location("ftp://example.com/pub/", None).is_err());
        assert!(location("ftp://example.com/", None).is_err());
        assert!(location("https://example.com/f", None).is_err());
        assert!(matches!(
            location("ftps://example.com:990/f", None),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn link_login_beats_account_and_accounts_stay_on_their_site() {
        let l = location("ftp://u:p@example.com/f", Some(login("example.com"))).unwrap();
        assert_eq!(l.username, "u");
        let l = location("ftp://files.example.com/f", Some(login("example.com"))).unwrap();
        assert_eq!((l.username.as_str(), l.password.as_str()), ("acct", "pw"));
        let l = location("ftp://other.org/f", Some(login("example.com"))).unwrap();
        assert_eq!(l.username, "anonymous");
    }

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

    #[test]
    fn replies_become_errors() {
        let e: Error = FtpError::ConnectionError(io::ErrorKind::ConnectionReset.into()).into();
        assert!(e.is_retryable());
        let reply = |code| suppaftp::types::Response {
            status: suppaftp::Status::from(code),
            body: format!("{code} Something happened").into_bytes(),
        };
        let e: Error = FtpError::UnexpectedResponse(reply(421)).into();
        assert!(e.is_retryable());
        let e: Error = FtpError::UnexpectedResponse(reply(550)).into();
        assert!(!e.is_retryable());
        assert_eq!(e.to_string(), "FTP server replied 550: Something happened");
    }
}
