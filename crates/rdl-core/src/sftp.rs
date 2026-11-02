//! SFTP downloads: one SSH connection per file, resumed by reading from an
//! offset.
//!
//! `sftp://[user[:password]@]host[:port]/path`: the path is absolute, and
//! `/~/` at its start means the login's home folder. The user comes from the
//! link, else the host's account, else it's the Windows user name (as
//! OpenSSH does). Logins try the password (the link's, or the account's for
//! the same user) and then the unencrypted keys in `%USERPROFILE%\.ssh`
//! (`id_ed25519`, `id_ecdsa`, `id_rsa`).
//!
//! Host keys are trusted on first use: the key a server shows the first
//! time is stored in `known_hosts`, and a different one later is refused
//! until the user forgets the old one. Proxies don't apply; the adapter
//! binding does.

use std::collections::HashMap;
use std::io::{self, SeekFrom};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use percent_encoding::percent_decode_str;
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use serde::Serialize;
use tokio::io::AsyncSeekExt;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use crate::accounts::Login;
use crate::db::Pool;
use crate::http::{Outcome, ProbeResult, Progress, SegmentConfig, filename};
use crate::single::{self, READ_TIMEOUT, Single};
use crate::{Error, Result};

/// Keys tried after the password, in this order.
const KEY_FILES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];
/// For the SSH handshake and the login, and for each SFTP request.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether `url` is an SFTP link.
pub fn is_sftp(url: &Url) -> bool {
    url.scheme() == "sftp"
}

/// Where a file is and how to log in for it.
#[derive(Clone)]
pub struct Location {
    url: Url,
    host: String,
    port: u16,
    path: String,
    username: String,
    password: Option<String>,
    /// The bound adapter's address, if downloads are bound to one.
    local: Option<IpAddr>,
    known: KnownHosts,
    /// Where the user's SSH keys are.
    key_dir: Option<PathBuf>,
}

impl std::fmt::Debug for Location {
    // Never the password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Location")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("path", &self.path)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "…"))
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

impl Location {
    /// `login` is the host's account.
    pub fn new(
        url: &Url,
        login: Option<Login>,
        local: Option<IpAddr>,
        known: KnownHosts,
    ) -> Result<Self> {
        let invalid = |why: &str| Error::InvalidUrl(format!("{url}: {why}"));
        if !is_sftp(url) {
            return Err(invalid("not an SFTP link"));
        }
        let host = match url.host() {
            Some(Host::Domain(name)) => name.to_owned(),
            Some(Host::Ipv4(ip)) => ip.to_string(),
            Some(Host::Ipv6(ip)) => ip.to_string(),
            None => return Err(invalid("no host")),
        };
        let path = decode(url.path());
        let path = match path.strip_prefix("/~/") {
            Some(home) => home.to_owned(),
            None => path,
        };
        if path.is_empty() || path.ends_with('/') || path == "/~" {
            return Err(invalid("a folder, not a file"));
        }
        let login = login.filter(|l| l.is_for(&host));
        let (username, password) = if !url.username().is_empty() {
            let user = decode(url.username());
            let password = url
                .password()
                .map(decode)
                .or_else(|| login.filter(|l| l.username == user).map(|l| l.password));
            (user, password)
        } else if let Some(login) = login {
            (login.username, Some(login.password))
        } else {
            (std::env::var("USERNAME").unwrap_or_default(), None)
        };
        Ok(Self {
            url: url.clone(),
            host,
            port: url.port().unwrap_or(22),
            path,
            username,
            password: password.filter(|p| !p.is_empty()),
            local,
            known,
            key_dir: dirs::home_dir().map(|home| home.join(".ssh")),
        })
    }

    /// Look for SSH keys in `dir` instead of `%USERPROFILE%\.ssh`.
    pub fn with_key_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.key_dir = dir;
        self
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

/// Log in and look the file up: its size and modification time.
pub async fn probe(location: &Location) -> Result<ProbeResult> {
    let session = connect(location).await?;
    let attrs = within(session.sftp.metadata(location.path.as_str())).await?;
    let _ = session.close().await;
    if attrs.is_dir() {
        return Err(Error::InvalidUrl(format!(
            "{}: a folder, not a file",
            location.url
        )));
    }
    let result = ProbeResult {
        final_url: location.url.clone(),
        size: attrs.size,
        accepts_ranges: true,
        etag: None,
        last_modified: attrs.mtime.map(|t| t.to_string()),
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
    let mut file = Single::start(
        part_path, final_path, probe.size, resume, config, progress, cancel,
    )
    .await?;
    let completed = loop {
        let before = file.written;
        match transfer(location, &mut file).await {
            Ok(completed) => break completed,
            Err(e) => {
                if !file.retry(e, before).await? {
                    break false;
                }
            }
        }
    };
    file.end(completed, final_path).await
}

/// One connection's worth: open the file, read on from where the part
/// file ends until it's done (`true`) or the download is cancelled.
async fn transfer(location: &Location, file: &mut Single<'_>) -> Result<bool> {
    let session = tokio::select! {
        _ = file.cancel.cancelled() => return Ok(false),
        session = connect(location) => session?,
    };
    if file.is_whole() {
        let _ = session.close().await;
        return Ok(true);
    }
    let mut remote = tokio::select! {
        _ = file.cancel.cancelled() => return Ok(false),
        remote = within(session.sftp.open(location.path.as_str())) => remote?,
    };
    remote
        .seek(SeekFrom::Start(file.written))
        .await
        .map_err(connection)?;
    let completed = file.copy(&mut remote, connection).await?;
    if completed {
        let _ = session.close().await;
    }
    Ok(completed)
}

/// An SFTP session and the SSH connection under it.
struct Session {
    sftp: SftpSession,
    ssh: Handle<HostCheck>,
}

impl Session {
    async fn close(self) -> Result<()> {
        let _ = within(self.sftp.close()).await;
        self.ssh
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await
            .map_err(ssh_error)
    }
}

/// Connect, check the host key, log in and start SFTP.
async fn connect(location: &Location) -> Result<Session> {
    let tcp = single::tcp_to(&location.host, location.port, location.local)
        .await
        .map_err(connection)?;
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(120)),
        keepalive_interval: Some(Duration::from_secs(15)),
        nodelay: true,
        ..client::Config::default()
    });
    let refusal = Arc::new(Mutex::new(None));
    let check = HostCheck {
        known: location.known.clone(),
        host: location.host.clone(),
        port: location.port,
        refusal: Arc::clone(&refusal),
    };
    let handshake = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        client::connect_stream(config, tcp, check),
    )
    .await
    .map_err(|_| connection(io::ErrorKind::TimedOut.into()))?;
    let mut ssh = match handshake {
        Ok(ssh) => ssh,
        // A refused host key says why better than the handshake error.
        Err(e) => {
            let refused = refusal.lock().unwrap_or_else(|e| e.into_inner()).take();
            return Err(refused.unwrap_or_else(|| ssh_error(e)));
        }
    };
    tokio::time::timeout(HANDSHAKE_TIMEOUT, log_in(&mut ssh, location))
        .await
        .map_err(|_| connection(io::ErrorKind::TimedOut.into()))??;
    let channel = ssh.channel_open_session().await.map_err(ssh_error)?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(ssh_error)?;
    let sftp = within(SftpSession::new(channel.into_stream())).await?;
    sftp.set_timeout(READ_TIMEOUT.as_secs());
    Ok(Session { sftp, ssh })
}

/// The password first, then the user's keys.
async fn log_in(ssh: &mut Handle<HostCheck>, location: &Location) -> Result<()> {
    let user = location.username.as_str();
    if let Some(password) = &location.password
        && ssh
            .authenticate_password(user, password)
            .await
            .map_err(ssh_error)?
            .success()
    {
        return Ok(());
    }
    let keys: Vec<PathBuf> = match &location.key_dir {
        Some(dir) => KEY_FILES.iter().map(|name| dir.join(name)).collect(),
        None => Vec::new(),
    };
    for path in keys.into_iter().filter(|path| path.is_file()) {
        let key = match russh::keys::load_secret_key(&path, None) {
            Ok(key) => key,
            Err(e) => {
                tracing::debug!(key = %path.display(), error = %e, "skipping SSH key");
                continue;
            }
        };
        let hash = ssh
            .best_supported_rsa_hash()
            .await
            .map_err(ssh_error)?
            .flatten();
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
        if ssh
            .authenticate_publickey(user, key)
            .await
            .map_err(ssh_error)?
            .success()
        {
            tracing::debug!(key = %path.display(), "logged in with SSH key");
            return Ok(());
        }
    }
    Err(Error::SshLogin {
        user: user.to_owned(),
        host: location.host.clone(),
    })
}

/// Checks the server's key against the known hosts.
struct HostCheck {
    known: KnownHosts,
    host: String,
    port: u16,
    /// Why the key was refused, for the error the caller sees.
    refusal: Arc<Mutex<Option<Error>>>,
}

impl client::Handler for HostCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let refused = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => {
                match self.known.check(&self.host, self.port, key).await {
                    Ok(()) => return Ok(true),
                    Err(e) => e,
                }
            }
            PublicKeyOrCertificate::Certificate(_) => {
                Error::Sftp("the server offered a host certificate, which isn't supported".into())
            }
        };
        tracing::warn!(host = %self.host, error = %refused, "host key refused");
        *self.refusal.lock().unwrap_or_else(|e| e.into_inner()) = Some(refused);
        Ok(false)
    }
}

/// A host key trusted for a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    /// OpenSSH's form: `SHA256:…`.
    pub fingerprint: String,
    /// Unix time in milliseconds.
    pub added_at: i64,
}

/// The host keys SFTP servers showed first, in the database (or in memory
/// without one).
#[derive(Clone)]
pub struct KnownHosts {
    pool: Option<Pool>,
    memory: Arc<Mutex<HashMap<(String, u16), KnownHost>>>,
}

impl KnownHosts {
    pub fn new(pool: Option<Pool>) -> Self {
        Self {
            pool,
            memory: Arc::default(),
        }
    }

    pub async fn list(&self) -> Result<Vec<KnownHost>> {
        let Some(pool) = &self.pool else {
            let memory = self.memory.lock().unwrap_or_else(|e| e.into_inner());
            return Ok(memory.values().cloned().collect());
        };
        let rows: Vec<(String, i64, String, String, i64)> = sqlx::query_as(
            "SELECT host, port, algorithm, fingerprint, added_at FROM known_hosts
             ORDER BY host, port",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(host, port, algorithm, fingerprint, added_at)| KnownHost {
                host,
                port: port as u16,
                algorithm,
                fingerprint,
                added_at,
            })
            .collect())
    }

    /// Stop trusting `host`'s key: the next connection trusts whatever it shows.
    pub async fn forget(&self, host: &str, port: u16) -> Result<()> {
        let Some(pool) = &self.pool else {
            self.memory
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&(host.to_owned(), port));
            return Ok(());
        };
        sqlx::query("DELETE FROM known_hosts WHERE host = ? AND port = ?")
            .bind(host)
            .bind(port as i64)
            .execute(pool)
            .await?;
        tracing::info!(%host, port, "forgot SFTP host key");
        Ok(())
    }

    /// `Ok` if `key` is the one known for `host`, or the first one seen
    /// (it's then remembered); `HostKeyChanged` if another one is known.
    async fn check(&self, host: &str, port: u16, key: &PublicKey) -> Result<()> {
        let seen = KnownHost {
            host: host.to_owned(),
            port,
            algorithm: key.algorithm().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            added_at: chrono::Utc::now().timestamp_millis(),
        };
        let known = match &self.pool {
            Some(pool) => {
                // The first key wins if two connections race to add one.
                sqlx::query(
                    "INSERT INTO known_hosts (host, port, algorithm, fingerprint) VALUES (?, ?, ?, ?)
                     ON CONFLICT (host, port) DO NOTHING",
                )
                .bind(host)
                .bind(port as i64)
                .bind(&seen.algorithm)
                .bind(&seen.fingerprint)
                .execute(pool)
                .await?;
                sqlx::query_scalar::<_, String>(
                    "SELECT fingerprint FROM known_hosts WHERE host = ? AND port = ?",
                )
                .bind(host)
                .bind(port as i64)
                .fetch_one(pool)
                .await?
            }
            None => self
                .memory
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry((host.to_owned(), port))
                .or_insert_with(|| seen.clone())
                .fingerprint
                .clone(),
        };
        if known == seen.fingerprint {
            Ok(())
        } else {
            Err(Error::HostKeyChanged {
                host: format!("{host}:{port}"),
                known,
                offered: seen.fingerprint,
            })
        }
    }
}

/// An SFTP request, given [`READ_TIMEOUT`] to answer.
async fn within<T>(request: impl Future<Output = std::result::Result<T, SftpError>>) -> Result<T> {
    match tokio::time::timeout(READ_TIMEOUT, request).await {
        Ok(result) => result.map_err(Error::from),
        Err(_) => Err(connection(io::ErrorKind::TimedOut.into())),
    }
}

fn connection(e: io::Error) -> Error {
    Error::SshConnection(e.to_string())
}

fn ssh_error(e: russh::Error) -> Error {
    use russh::Error as E;
    match e {
        E::IO(_)
        | E::Disconnect
        | E::HUP
        | E::ConnectionTimeout
        | E::KeepaliveTimeout
        | E::InactivityTimeout
        | E::Elapsed(_)
        | E::SendError
        | E::RecvError => Error::SshConnection(e.to_string()),
        other => Error::Sftp(format!("SSH: {other}")),
    }
}

impl From<SftpError> for Error {
    fn from(e: SftpError) -> Self {
        match e {
            SftpError::Status(status) => Error::SftpStatus {
                code: status.status_code as u32,
                message: if status.error_message.trim().is_empty() {
                    status.status_code.to_string()
                } else {
                    status.error_message
                },
            },
            SftpError::IO(e) => Error::SshConnection(e),
            SftpError::Timeout => Error::SshConnection("timed out".into()),
            other => Error::Sftp(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(url: &str, login: Option<Login>) -> Result<Location> {
        Location::new(
            &Url::parse(url).unwrap(),
            login,
            None,
            KnownHosts::new(None),
        )
    }

    fn login(site: &str, user: &str) -> Login {
        Login {
            site: site.into(),
            username: user.into(),
            password: "pw".into(),
        }
    }

    #[test]
    fn locations_from_links() {
        let l = location("sftp://me:p%40ss@files.example.com/srv/a%20b.iso", None).unwrap();
        assert_eq!((l.host.as_str(), l.port), ("files.example.com", 22));
        assert_eq!(l.path, "/srv/a b.iso");
        assert_eq!(l.filename(), "a b.iso");
        assert_eq!(
            (l.username.as_str(), l.password.as_deref()),
            ("me", Some("p@ss"))
        );
        assert!(!format!("{l:?}").contains("p@ss"));

        let l = location("sftp://me@[::1]:2222/~/backup.tar", None).unwrap();
        assert_eq!((l.host.as_str(), l.port), ("::1", 2222));
        assert_eq!(l.path, "backup.tar");
        assert_eq!(l.password, None);

        assert!(location("sftp://example.com/srv/", None).is_err());
        assert!(location("sftp://example.com/", None).is_err());
        assert!(location("ftp://example.com/f", None).is_err());
    }

    #[test]
    fn logins_come_from_the_link_then_the_account() {
        let account = Some(login("example.com", "acct"));
        let l = location("sftp://example.com/f", account.clone()).unwrap();
        assert_eq!(
            (l.username.as_str(), l.password.as_deref()),
            ("acct", Some("pw"))
        );
        // The account's password goes with its own user only.
        let l = location("sftp://acct@example.com/f", account.clone()).unwrap();
        assert_eq!(l.password.as_deref(), Some("pw"));
        let l = location("sftp://other@example.com/f", account.clone()).unwrap();
        assert_eq!((l.username.as_str(), l.password), ("other", None));
        // Another site's account isn't used.
        let l = location("sftp://example.org/f", account).unwrap();
        assert_eq!(l.password, None);
        assert_eq!(l.username, std::env::var("USERNAME").unwrap_or_default());
    }

    #[tokio::test]
    async fn host_keys_are_trusted_on_first_use() {
        let known = KnownHosts::new(None);
        let key = |base64| PublicKey::from_openssh(&format!("ssh-ed25519 {base64}")).unwrap();
        let first = key("AAAAC3NzaC1lZDI1NTE5AAAAIH3aSSYILBDh9aT/N4s3+1tmhzJ0xXnqG/Q0+nYjmo1m");
        let second = key("AAAAC3NzaC1lZDI1NTE5AAAAIIjwka6Z2oYDI/XAOhO7mjygpySW6CmkLj8MD8cPcRT2");
        known.check("h", 22, &first).await.unwrap();
        known.check("h", 22, &first).await.unwrap();
        // Another port is another server.
        known.check("h", 2222, &second).await.unwrap();
        let changed = known.check("h", 22, &second).await.unwrap_err();
        assert!(matches!(changed, Error::HostKeyChanged { .. }), "{changed}");
        assert!(!changed.is_retryable());
        assert_eq!(known.list().await.unwrap().len(), 2);
        known.forget("h", 22).await.unwrap();
        known.check("h", 22, &second).await.unwrap();
    }
}
