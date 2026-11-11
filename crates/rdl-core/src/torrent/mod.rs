//! BitTorrent through librqbit: one session for the whole app, started on the
//! first torrent and kept until shutdown. The session's state (the torrent
//! list with fast-resume data, and the DHT table) lives in one folder, so a
//! portable build keeps it in its own data folder.
//!
//! Network settings reach the session where librqbit allows it. The listening
//! socket is bound to the chosen adapter's address. A SOCKS5 default proxy
//! carries peer connections (and then DHT stays off, since its UDP would go
//! around the proxy). Outgoing sockets can't be bound to an address on
//! Windows, so the adapter binding doesn't pin peer connections; the
//! kill-switch works by pausing the torrents.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use librqbit::api::TorrentIdOrHash;
use librqbit::dht::DhtPersistenceConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ConnectionOptions, DhtSessionConfig,
    ListenerOptions, Session, SessionOptions, SessionPersistenceConfig,
};
use tokio::sync::OnceCell;
use url::Url;

pub use librqbit::{ManagedTorrent, TorrentStats, TorrentStatsState};

use crate::network::Network;
use crate::proxy::{ProxyChoice, ProxyKind};
use crate::{Error, Result};

/// Where the engine keeps its state, and what it is for.
#[derive(Debug, Clone)]
pub struct TorrentConfig {
    /// Session state (`session/`, `dht.json`) lives here.
    pub state_dir: PathBuf,
    /// Tests only. When not empty, the session listens on loopback, runs
    /// without DHT and trackers, and every torrent asks these addresses for
    /// peers. Nothing else changes.
    pub test_peers: Vec<SocketAddr>,
}

/// Where a torrent comes from.
#[derive(Debug, Clone)]
pub enum TorrentSource {
    Magnet(String),
    /// The contents of a `.torrent` file.
    Bytes(Vec<u8>),
}

/// What a torrent is, read before it is added.
#[derive(Debug, Clone)]
pub struct Inspection {
    /// 40 hex digits.
    pub info_hash: String,
    pub name: String,
    /// Total size of all its files, in bytes.
    pub size: u64,
    pub files: usize,
    /// The `.torrent` contents (for a magnet, the metadata that was fetched).
    pub torrent: Vec<u8>,
}

/// Whether a link is a torrent: a `magnet:` link, or an http(s) or file link
/// to a `.torrent` file.
pub fn is_torrent_link(url: &Url) -> bool {
    match url.scheme() {
        "magnet" => true,
        "http" | "https" | "file" => url.path().to_ascii_lowercase().ends_with(".torrent"),
        _ => false,
    }
}

/// Cheap to clone; all clones share one session.
#[derive(Clone)]
pub struct TorrentEngine {
    inner: Arc<Inner>,
}

struct Inner {
    config: TorrentConfig,
    network: Network,
    /// Filled on first use; a failed start is retried on the next use.
    session: OnceCell<Arc<Session>>,
    /// The download speed limit for the whole session, bytes per second.
    download_limit: Mutex<Option<u64>>,
}

impl TorrentEngine {
    /// Nothing is started yet; the session starts on first use.
    pub fn new(config: TorrentConfig, network: Network) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                network,
                session: OnceCell::new(),
                download_limit: Mutex::new(None),
            }),
        }
    }

    fn test_mode(&self) -> bool {
        !self.inner.config.test_peers.is_empty()
    }

    async fn session(&self) -> Result<Arc<Session>> {
        let session = self.inner.session.get_or_try_init(|| self.start()).await?;
        Ok(Arc::clone(session))
    }

    async fn start(&self) -> Result<Arc<Session>> {
        let config = &self.inner.config;
        let test = self.test_mode();
        // Refuses while the bound adapter is down, like every other connection.
        let local = self.inner.network.local_address()?;
        let proxy = if test { None } else { self.socks_proxy() };
        let ip = if test {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            local.unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED))
        };
        let port = if test { 0 } else { free_port(ip) };

        tokio::fs::create_dir_all(&config.state_dir)
            .await
            .map_err(Error::Io)?;
        // DHT's UDP would bypass a proxy, so it stays off behind one.
        let dht = (!test && proxy.is_none()).then(|| DhtSessionConfig {
            persistence: Some(DhtPersistenceConfig {
                config_filename: Some(config.state_dir.join("dht.json")),
                ..Default::default()
            }),
            ..Default::default()
        });
        let options = SessionOptions {
            dht,
            disable_trackers: test,
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(config.state_dir.join("session")),
            }),
            listen: Some(ListenerOptions {
                listen_addr: SocketAddr::new(ip, port),
                ..Default::default()
            }),
            connect: proxy.map(|url| ConnectionOptions {
                proxy_url: Some(url),
                ..Default::default()
            }),
            ..Default::default()
        };
        let session = Session::new_with_opts(config.state_dir.clone(), options)
            .await
            .map_err(torrent_error)?;
        session
            .ratelimits
            .set_download_bps(bps(*lock(&self.inner.download_limit)));
        tracing::info!(address = %ip, port, "torrent session started");
        Ok(session)
    }

    /// The SOCKS5 proxy for peer connections: the default proxy, if it is one.
    /// Per-download proxies don't apply to torrents (one session serves all).
    fn socks_proxy(&self) -> Option<String> {
        let network = &self.inner.network;
        let id = network.settings().default?;
        let profile = network.profiles().into_iter().find(|p| p.id == id)?;
        if profile.kind != ProxyKind::Socks5 {
            return None;
        }
        // The route is picked by id; the URL it names is not used.
        let placeholder = Url::parse("https://torrent.invalid/").ok()?;
        let route = network.route(&placeholder, ProxyChoice::Profile { id })?;
        Some(route.url.replacen("socks5h://", "socks5://", 1))
    }

    /// The options every add shares: the test peers, if any.
    fn add_options(&self) -> AddTorrentOptions {
        let peers = &self.inner.config.test_peers;
        AddTorrentOptions {
            initial_peers: (!peers.is_empty()).then(|| peers.clone()),
            disable_trackers: self.test_mode(),
            ..Default::default()
        }
    }

    /// Read a torrent's name, size and files without downloading anything.
    /// A magnet link waits for its metadata from the peers, however long
    /// that takes; the caller cancels by dropping the future.
    pub async fn inspect(&self, source: TorrentSource) -> Result<Inspection> {
        let session = self.session().await?;
        let mut options = self.add_options();
        options.list_only = true;
        let response = session
            .add_torrent(source.into_add(), Some(options))
            .await
            .map_err(torrent_error)?;
        let AddTorrentResponse::ListOnly(list) = response else {
            return Err(Error::Torrent("unexpected reply from the engine".into()));
        };
        let info = &list.info;
        Ok(Inspection {
            info_hash: list.info_hash.as_string(),
            name: info.name().map(|n| n.into_owned()).unwrap_or_default(),
            size: info.iter_file_details().map(|f| f.len).sum(),
            files: info.iter_file_details().count(),
            torrent: list.torrent_bytes.to_vec(),
        })
    }

    /// Add a torrent whose files go into `output_folder`, and return its
    /// handle. A torrent already in the session returns its existing handle.
    /// Existing files are checked and reused.
    pub async fn add(
        &self,
        source: TorrentSource,
        output_folder: &Path,
        paused: bool,
    ) -> Result<Arc<ManagedTorrent>> {
        let session = self.session().await?;
        let mut options = self.add_options();
        options.paused = paused;
        options.overwrite = true;
        options.output_folder = Some(output_folder.to_string_lossy().into_owned());
        let response = session
            .add_torrent(source.into_add(), Some(options))
            .await
            .map_err(torrent_error)?;
        match response {
            AddTorrentResponse::Added(_, handle)
            | AddTorrentResponse::AlreadyManaged(_, handle) => Ok(handle),
            AddTorrentResponse::ListOnly(_) => {
                Err(Error::Torrent("unexpected reply from the engine".into()))
            }
        }
    }

    /// The torrent with this info hash, if the session has it. Starts the
    /// session if needed, so torrents from an earlier run are found.
    pub async fn find(&self, info_hash: &str) -> Result<Option<Arc<ManagedTorrent>>> {
        let session = self.session().await?;
        Ok(session.with_torrents(|torrents| {
            for (_, handle) in torrents {
                if handle.info_hash().as_string() == info_hash {
                    return Some(Arc::clone(handle));
                }
            }
            None
        }))
    }

    /// The torrent a download resumes with. Test mode only: a torrent restored
    /// from the session file has lost its test peers (librqbit doesn't store
    /// them), so it is taken out, its files kept, and added again from its
    /// source. Outside tests this is just [`TorrentEngine::find`].
    pub async fn find_for_resume(&self, info_hash: &str) -> Result<Option<Arc<ManagedTorrent>>> {
        let Some(handle) = self.find(info_hash).await? else {
            return Ok(None);
        };
        if !self.test_mode() {
            return Ok(Some(handle));
        }
        self.forget(info_hash, false).await?;
        Ok(None)
    }

    pub async fn pause(&self, handle: &Arc<ManagedTorrent>) -> Result<()> {
        if handle.is_paused() {
            return Ok(());
        }
        let session = self.session().await?;
        session.pause(handle).await.map_err(torrent_error)
    }

    pub async fn resume(&self, handle: &Arc<ManagedTorrent>) -> Result<()> {
        if !handle.is_paused() {
            return Ok(());
        }
        let session = self.session().await?;
        session.unpause(handle).await.map_err(torrent_error)
    }

    /// Take a torrent out of the session. With `delete_files`, its files
    /// go too. A torrent the session doesn't have is not an error.
    pub async fn forget(&self, info_hash: &str, delete_files: bool) -> Result<()> {
        let Some(handle) = self.find(info_hash).await? else {
            return Ok(());
        };
        let session = self.session().await?;
        session
            .delete(TorrentIdOrHash::Id(handle.id()), delete_files)
            .await
            .map_err(torrent_error)
    }

    /// The download speed limit for all torrents, in bytes per second; `None`
    /// lifts it. Takes effect at once, and at the next start if the session
    /// isn't running yet.
    pub fn set_download_limit(&self, bytes_per_second: Option<u64>) {
        *lock(&self.inner.download_limit) = bytes_per_second;
        if let Some(session) = self.inner.session.get() {
            session.ratelimits.set_download_bps(bps(bytes_per_second));
        }
    }

    /// Stop the session if it was started: torrents pause and state is
    /// flushed. Safe to call more than once.
    pub async fn stop(&self) {
        if let Some(session) = self.inner.session.get() {
            session.stop().await;
        }
    }
}

impl TorrentSource {
    fn into_add(self) -> AddTorrent<'static> {
        match self {
            TorrentSource::Magnet(link) => AddTorrent::from_url(link),
            TorrentSource::Bytes(bytes) => AddTorrent::from_bytes(bytes),
        }
    }
}

/// The first free port in 42000 to 42020 on `ip` for incoming peers, or 0
/// (the OS picks) if none is free.
fn free_port(ip: IpAddr) -> u16 {
    (42000..=42020)
        .find(|port| TcpListener::bind(SocketAddr::new(ip, *port)).is_ok())
        .unwrap_or(0)
}

/// A session-wide limit as librqbit takes it; `None` or zero means unlimited.
fn bps(limit: Option<u64>) -> Option<NonZeroU32> {
    limit.and_then(|l| NonZeroU32::new(l.min(u64::from(u32::MAX)) as u32))
}

fn torrent_error(e: impl std::fmt::Display) -> Error {
    Error::Torrent(format!("{e:#}"))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torrent_links_are_recognised() {
        let yes = [
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            "https://example.com/files/ubuntu.torrent",
            "HTTP://example.com/Ubuntu.TORRENT",
            "file:///C:/Downloads/a.torrent",
        ];
        for link in yes {
            assert!(is_torrent_link(&Url::parse(link).unwrap()), "{link}");
        }
        let no = [
            "https://example.com/file.zip",
            "https://example.com/torrent/page",
            "ftp://example.com/a.torrent",
        ];
        for link in no {
            assert!(!is_torrent_link(&Url::parse(link).unwrap()), "{link}");
        }
    }

    #[test]
    fn limits_clamp_and_zero_is_unlimited() {
        assert_eq!(bps(None), None);
        assert_eq!(bps(Some(0)), None);
        assert_eq!(bps(Some(1024)), NonZeroU32::new(1024));
        assert_eq!(bps(Some(u64::MAX)), NonZeroU32::new(u32::MAX));
    }
}
