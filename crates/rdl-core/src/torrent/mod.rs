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
//!
//! Metadata (the `.torrent` contents of a torrent) is kept under the state
//! folder in `metadata/<info hash>.torrent`. A magnet whose metadata is there
//! is read from that copy, so checking a magnet and downloading it fetch the
//! metadata from the peers only once. Files the app imports (a dropped or
//! opened `.torrent`) are copied to `files/<info hash>.torrent`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use librqbit::api::TorrentIdOrHash;
use librqbit::dht::DhtPersistenceConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ConnectionOptions, DhtSessionConfig,
    ListenerOptions, Session, SessionOptions, SessionPersistenceConfig, ValidatedTorrentMetaV1Info,
    torrent_from_bytes,
};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use url::Url;

pub use librqbit::{ManagedTorrent, TorrentStats, TorrentStatsState};

use crate::http::{self, DownloadSpec};
use crate::network::Network;
use crate::proxy::{ProxyChoice, ProxyKind};
use crate::request::RequestContext;
use crate::{Error, Result};

/// Largest `.torrent` file that is read or fetched.
pub const MAX_TORRENT_FILE: usize = 16 * 1024 * 1024;

/// Where the engine keeps its state, and what it is for.
#[derive(Debug, Clone)]
pub struct TorrentConfig {
    /// Session state (`session/`, `dht.json`, `metadata/`, `files/`) lives here.
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

/// One file of a torrent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    /// Its position in the torrent: the number file selection uses.
    pub index: usize,
    /// Relative to the torrent's folder, `/`-separated.
    pub path: String,
    pub size: u64,
}

/// What a torrent is: name, size and files, as read from its metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct TorrentInfo {
    /// 40 lowercase hex digits.
    pub info_hash: String,
    pub name: String,
    /// Total size of all its files, in bytes.
    pub size: u64,
    pub files: Vec<TorrentFile>,
}

/// Which files of a torrent to download. `None` is all of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct TorrentOptions {
    /// Indices into the torrent's files (see [`TorrentFile::index`]).
    pub only_files: Option<Vec<usize>>,
}

/// Total size of the files that `only_files` selects (all when `None`).
pub fn selected_size(files: &[TorrentFile], only_files: Option<&[usize]>) -> u64 {
    files
        .iter()
        .filter(|f| only_files.is_none_or(|selected| selected.contains(&f.index)))
        .map(|f| f.size)
        .sum()
}

/// What a torrent is, read before it is added.
#[derive(Debug, Clone)]
pub struct Inspection {
    /// 40 lowercase hex digits.
    pub info_hash: String,
    pub name: String,
    /// Total size of all its files, in bytes.
    pub size: u64,
    pub files: Vec<TorrentFile>,
    /// The `.torrent` contents (for a magnet, the metadata that was fetched).
    pub torrent: Vec<u8>,
}

impl Inspection {
    /// The part of this the UI and the download record keep. A torrent
    /// without a name is shown by its info hash.
    pub fn info(&self) -> TorrentInfo {
        let name = if self.name.trim().is_empty() {
            self.info_hash.clone()
        } else {
            self.name.clone()
        };
        TorrentInfo {
            info_hash: self.info_hash.clone(),
            name,
            size: self.size,
            files: self.files.clone(),
        }
    }
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

/// The info hash of a magnet link (its `xt=urn:btih:` parameter), as 40
/// lowercase hex digits. `None` if the link has none.
pub fn magnet_hash(url: &Url) -> Option<String> {
    url.query_pairs().find_map(|(key, value)| {
        (key == "xt")
            .then(|| {
                value
                    .strip_prefix("urn:btih:")
                    .and_then(normalize_info_hash)
            })
            .flatten()
    })
}

/// An info hash written as 40 hex digits or 32 base32 characters, as 40
/// lowercase hex digits. `None` for anything else.
pub fn normalize_info_hash(text: &str) -> Option<String> {
    let text = text.trim();
    if text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(text.to_ascii_lowercase());
    }
    if text.len() == 32 {
        return base32_decode(text).map(|bytes| hex(&bytes));
    }
    None
}

/// RFC 4648 base32 without padding, 20 bytes out of 32 characters.
fn base32_decode(text: &str) -> Option<[u8; 20]> {
    let mut out = [0u8; 20];
    let mut filled = 0;
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            *out.get_mut(filled)? = (buffer >> bits) as u8;
            filled += 1;
            buffer &= (1 << bits) - 1;
        }
    }
    (filled == out.len()).then_some(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

    /// Where the metadata copy of a torrent is kept.
    fn metadata_path(&self, info_hash: &str) -> PathBuf {
        self.inner
            .config
            .state_dir
            .join("metadata")
            .join(format!("{info_hash}.torrent"))
    }

    /// Read a torrent's name, size and files without downloading anything.
    ///
    /// A `.torrent` is read locally. A magnet is answered from the metadata
    /// copy when there is one, and otherwise waits for its metadata from the
    /// peers, however long that takes; the caller cancels by dropping the
    /// future. Whatever is read is kept as the metadata copy.
    pub async fn inspect(&self, source: TorrentSource) -> Result<Inspection> {
        let inspection = match source {
            TorrentSource::Bytes(bytes) => parse_torrent(bytes)?,
            TorrentSource::Magnet(link) => {
                let cached = Url::parse(&link)
                    .ok()
                    .and_then(|url| magnet_hash(&url))
                    .and_then(|hash| self.inspect_cached(&hash));
                if let Some(cached) = cached {
                    return Ok(cached);
                }
                self.fetch_metadata(link).await?
            }
        };
        self.keep_metadata(&inspection).await;
        Ok(inspection)
    }

    /// A torrent's metadata from the peers, through a list-only add.
    async fn fetch_metadata(&self, link: String) -> Result<Inspection> {
        let session = self.session().await?;
        let mut options = self.add_options();
        options.list_only = true;
        let response = session
            .add_torrent(AddTorrent::from_url(link), Some(options))
            .await
            .map_err(torrent_error)?;
        let AddTorrentResponse::ListOnly(list) = response else {
            return Err(Error::Torrent("unexpected reply from the engine".into()));
        };
        let (name, files) = describe(&list.info);
        let size = files.iter().map(|f| f.size).sum();
        Ok(Inspection {
            info_hash: list.info_hash.as_string(),
            name,
            size,
            files,
            torrent: list.torrent_bytes.to_vec(),
        })
    }

    /// The metadata copy of a torrent, if one was kept. Reads a local file
    /// only: no session, no network.
    pub fn inspect_cached(&self, info_hash: &str) -> Option<Inspection> {
        let hash = normalize_info_hash(info_hash)?;
        let bytes = std::fs::read(self.metadata_path(&hash)).ok()?;
        let inspection = parse_torrent(bytes).ok()?;
        (inspection.info_hash == hash).then_some(inspection)
    }

    /// Keep a torrent's metadata copy. A failure is logged, not an error: the
    /// copy is only a saving.
    async fn keep_metadata(&self, inspection: &Inspection) {
        let path = self.metadata_path(&inspection.info_hash);
        if let Err(e) = write_atomic(&path, &inspection.torrent).await {
            tracing::warn!(error = %e, "could not keep torrent metadata");
        }
    }

    /// Check a `.torrent` file's contents and copy them to
    /// `files/<info hash>.torrent`, so the original may be deleted later.
    /// Returns what it is and the copy's path. Doesn't start the session.
    pub async fn import_file(&self, bytes: Vec<u8>) -> Result<(Inspection, PathBuf)> {
        let inspection = parse_torrent(bytes)?;
        let path = self
            .inner
            .config
            .state_dir
            .join("files")
            .join(format!("{}.torrent", inspection.info_hash));
        write_atomic(&path, &inspection.torrent).await?;
        Ok((inspection, path))
    }

    /// Add a torrent whose files go into `output_folder`, and return its
    /// handle. `only_files` selects which files download (`None`: all). A
    /// torrent already in the session returns its existing handle. Existing
    /// files are checked and reused.
    pub async fn add(
        &self,
        source: TorrentSource,
        output_folder: &Path,
        paused: bool,
        only_files: Option<Vec<usize>>,
    ) -> Result<Arc<ManagedTorrent>> {
        let session = self.session().await?;
        let mut options = self.add_options();
        options.paused = paused;
        options.overwrite = true;
        options.output_folder = Some(output_folder.to_string_lossy().into_owned());
        options.only_files = only_files;
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

/// Parse the contents of a `.torrent` file, or say it isn't one.
fn parse_torrent(bytes: Vec<u8>) -> Result<Inspection> {
    let not_torrent = || Error::Torrent("That isn't a .torrent file".into());
    let (info_hash, name, files) = {
        let meta = torrent_from_bytes(&bytes).map_err(|_| not_torrent())?;
        let validated = meta.info.data.validate().map_err(|_| not_torrent())?;
        let (name, files) = describe(&validated);
        (meta.info_hash.as_string(), name, files)
    };
    let size = files.iter().map(|f| f.size).sum();
    Ok(Inspection {
        info_hash,
        name,
        size,
        files,
        torrent: bytes,
    })
}

/// A torrent's name and its files, from validated metadata.
fn describe<B: AsRef<[u8]>>(info: &ValidatedTorrentMetaV1Info<B>) -> (String, Vec<TorrentFile>) {
    let name = info.name().map(|n| n.into_owned()).unwrap_or_default();
    let files = info
        .iter_file_details()
        .enumerate()
        .map(|(index, details)| TorrentFile {
            index,
            path: details
                .filename
                .iter_components()
                .map(|part| part.into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            size: details.len,
        })
        .collect();
    (name, files)
}

/// Write `bytes` to `path` through a temporary file, creating the folder.
async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temp = path.with_extension("torrent.part");
    tokio::fs::write(&temp, bytes).await?;
    tokio::fs::rename(&temp, path).await?;
    Ok(())
}

/// The `.torrent` contents at `url`, over HTTP(S), with the connection and
/// account the link would use. At most [`MAX_TORRENT_FILE`] bytes.
pub(crate) async fn fetch_http(
    network: &Network,
    url: &Url,
    proxy: ProxyChoice,
    request: RequestContext,
) -> Result<Vec<u8>> {
    let client = network.client(url, proxy)?;
    let (request, login) = network.with_account(url, request);
    let spec = DownloadSpec::with_request(url.clone(), request).with_login(login);
    let response = http::get(&client, &spec, url, &HeaderMap::new()).await?;
    http::check_status(response.status(), url)?;
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_TORRENT_FILE {
        return Err(Error::Torrent(
            "the .torrent file is too large to be one".into(),
        ));
    }
    Ok(bytes.to_vec())
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

    #[test]
    fn info_hashes_are_normalised_to_lowercase_hex() {
        let hex = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(normalize_info_hash(hex).as_deref(), Some(hex));
        assert_eq!(
            normalize_info_hash(&hex.to_uppercase()).as_deref(),
            Some(hex)
        );
        // 20 zero bytes are 32 'A's in base32; 20 0xff bytes are 32 '7's.
        assert_eq!(
            normalize_info_hash("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").as_deref(),
            Some("0".repeat(40).as_str())
        );
        assert_eq!(
            normalize_info_hash("77777777777777777777777777777777").as_deref(),
            Some("f".repeat(40).as_str())
        );
        // Lowercase base32 is accepted too.
        assert_eq!(
            normalize_info_hash("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").as_deref(),
            Some("0".repeat(40).as_str())
        );
        assert_eq!(normalize_info_hash("0123"), None);
        assert_eq!(normalize_info_hash(&"z".repeat(40)), None);
        assert_eq!(
            normalize_info_hash("1111111111111111111111111111111x"),
            None
        );
    }

    #[test]
    fn magnet_hashes_come_from_the_btih_parameter() {
        let base32 =
            Url::parse("magnet:?dn=x&xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
        assert_eq!(
            magnet_hash(&base32).as_deref(),
            Some("0".repeat(40).as_str())
        );
        let hex = Url::parse(
            "magnet:?xt=urn:btmh:1220aa&xt=urn:btih:0123456789ABCDEF0123456789abcdef01234567",
        )
        .unwrap();
        assert_eq!(
            magnet_hash(&hex).as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        let none = Url::parse("magnet:?dn=nothing&xt=urn:sha1:0123").unwrap();
        assert_eq!(magnet_hash(&none), None);
    }

    #[test]
    fn selected_size_counts_only_the_chosen_files() {
        let files = [
            TorrentFile {
                index: 0,
                path: "a".into(),
                size: 10,
            },
            TorrentFile {
                index: 1,
                path: "b".into(),
                size: 100,
            },
        ];
        assert_eq!(selected_size(&files, None), 110);
        assert_eq!(selected_size(&files, Some(&[1])), 100);
        assert_eq!(selected_size(&files, Some(&[])), 0);
    }
}
