//! The LinkGrabber: a staging area where links are checked (name, size,
//! availability) and grouped into packages before they are queued.
//!
//! Links come from paste, the clipboard watcher, drag-drop or (later) the
//! browser. Each is stored at once and probed in the background with bounded
//! concurrency. A link that turns out to be a web page (or can't be probed)
//! is handed to yt-dlp, which recognises video sites; playlists expand into
//! one link per video (see `media`). User edits (file and package names, folders) always win over
//! what a probe finishes with later. Confirming hands links to the
//! [`Manager`] as packages and removes them from here.

mod links;
mod media;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, broadcast};
use url::Url;

use crate::db::Pool;
use crate::http::{self, DownloadSpec, filename};
use crate::manager::{DEFAULT_QUEUE, DownloadKind, Manager, NewDownload, NewPackage, PackageId};
use crate::tools::Tools;
use crate::ytdlp::{MediaInfo, YtDlpOptions};
use crate::{Error, Result};

pub use links::{archive_set_name, default_package_name, extract_links};
pub use media::MediaSummary;

pub type LinkId = i64;

/// How many links are probed at the same time.
const PROBE_CONCURRENCY: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum Availability {
    Unknown,
    Checking,
    Online,
    /// The server says the file is gone (404, 410) or refuses access.
    Offline,
    /// The check itself failed (network, timeout, server error); may be temporary.
    Error,
}

impl Availability {
    fn as_str(self) -> &'static str {
        match self {
            Availability::Unknown => "unknown",
            Availability::Checking => "checking",
            Availability::Online => "online",
            Availability::Offline => "offline",
            Availability::Error => "error",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "checking" => Availability::Checking,
            "online" => Availability::Online,
            "offline" => Availability::Offline,
            "error" => Availability::Error,
            _ => Availability::Unknown,
        }
    }
}

/// Where links came from, so the UI can say "3 links from the clipboard".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum LinkSource {
    Manual,
    Clipboard,
    Drop,
    /// Expanded from a playlist link already in the LinkGrabber.
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GrabbedLink {
    pub id: LinkId,
    pub url: String,
    pub host: String,
    pub filename: Option<String>,
    pub size: Option<u64>,
    pub availability: Availability,
    pub error: Option<String>,
    pub package: String,
    /// `None` means the default download folder.
    pub dest_dir: Option<PathBuf>,
    pub kind: DownloadKind,
    /// What yt-dlp found, for `Ytdlp` links.
    pub media: Option<MediaSummary>,
    /// Format choice, for `Ytdlp` links.
    pub ytdlp: Option<YtDlpOptions>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GrabberEvent {
    Added {
        links: Vec<GrabbedLink>,
        source: LinkSource,
    },
    Updated {
        link: Box<GrabbedLink>,
    },
    Removed {
        ids: Vec<LinkId>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AddOutcome {
    pub added: Vec<GrabbedLink>,
    /// Links skipped because they are already in the LinkGrabber.
    pub duplicates: usize,
}

#[derive(Clone)]
pub struct LinkGrabber {
    inner: Arc<Inner>,
}

struct Inner {
    pool: Pool,
    client: reqwest::Client,
    events: broadcast::Sender<GrabberEvent>,
    probes: Arc<Semaphore>,
    tools: Option<Tools>,
}

#[derive(Debug, sqlx::FromRow)]
struct LinkRow {
    id: i64,
    source_url: String,
    kind: Option<String>,
    filename: Option<String>,
    size: Option<i64>,
    availability: String,
    error: Option<String>,
    proposed_package: Option<String>,
    dest_dir: Option<String>,
    handler_opts_json: Option<String>,
    info_json: Option<String>,
}

impl LinkRow {
    fn kind(&self) -> DownloadKind {
        match self.kind.as_deref() {
            Some("ytdlp") => DownloadKind::Ytdlp,
            _ => DownloadKind::Http,
        }
    }

    fn media_info(&self) -> Option<MediaInfo> {
        (self.kind() == DownloadKind::Ytdlp)
            .then(|| serde_json::from_str(self.info_json.as_deref()?).ok())
            .flatten()
    }

    fn ytdlp_options(&self) -> Option<YtDlpOptions> {
        (self.kind() == DownloadKind::Ytdlp).then(|| {
            self.handler_opts_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default()
        })
    }

    fn into_link(self) -> GrabbedLink {
        let kind = self.kind();
        let media = self.media_info().map(|info| MediaSummary::from_info(&info));
        let ytdlp = self.ytdlp_options();
        let host = Url::parse(&self.source_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        GrabbedLink {
            id: self.id,
            host,
            url: self.source_url,
            filename: self.filename,
            size: self.size.map(|s| s as u64),
            availability: Availability::parse(&self.availability),
            error: self.error,
            package: self.proposed_package.unwrap_or_else(|| "Links".to_owned()),
            dest_dir: self.dest_dir.map(PathBuf::from),
            kind,
            media,
            ytdlp,
        }
    }
}

impl LinkGrabber {
    /// Load the grabber and re-check links whose probe didn't finish last run.
    /// Without `tools`, every link is checked as a plain file.
    pub async fn start(pool: Pool, user_agent: Option<&str>, tools: Option<Tools>) -> Result<Self> {
        let (events, _) = broadcast::channel(256);
        let grabber = Self {
            inner: Arc::new(Inner {
                client: http::client(user_agent)?,
                pool,
                events,
                probes: Arc::new(Semaphore::new(PROBE_CONCURRENCY)),
                tools,
            }),
        };
        let pending: Vec<LinkRow> = sqlx::query_as(
            "SELECT * FROM link_grabber WHERE availability IN ('unknown', 'checking') ORDER BY id",
        )
        .fetch_all(&grabber.inner.pool)
        .await?;
        for row in pending {
            grabber.spawn_probe(row.id, row.source_url, row.proposed_package);
        }
        Ok(grabber)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GrabberEvent> {
        self.inner.events.subscribe()
    }

    pub async fn list(&self) -> Result<Vec<GrabbedLink>> {
        let rows: Vec<LinkRow> = sqlx::query_as("SELECT * FROM link_grabber ORDER BY id")
            .fetch_all(&self.inner.pool)
            .await?;
        Ok(rows.into_iter().map(LinkRow::into_link).collect())
    }

    /// Stage links, skipping ones already here, and start checking them.
    pub async fn add(&self, urls: Vec<Url>, source: LinkSource) -> Result<AddOutcome> {
        let pool = &self.inner.pool;
        let existing: HashSet<String> = sqlx::query_scalar("SELECT source_url FROM link_grabber")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
        let requested = urls.len();
        let mut seen = HashSet::new();
        let fresh: Vec<Url> = urls
            .into_iter()
            .filter(|u| !existing.contains(u.as_str()) && seen.insert(u.to_string()))
            .collect();
        let duplicates = requested - fresh.len();

        let package = default_package_name(&fresh);
        let mut added = Vec::with_capacity(fresh.len());
        let mut tx = pool.begin().await?;
        for url in &fresh {
            let row: LinkRow = sqlx::query_as(
                "INSERT INTO link_grabber (source_url, kind, proposed_package) VALUES (?, 'http', ?) RETURNING *",
            )
            .bind(url.as_str())
            .bind(&package)
            .fetch_one(&mut *tx)
            .await?;
            added.push(row.into_link());
        }
        tx.commit().await?;

        if !added.is_empty() {
            tracing::info!(count = added.len(), ?source, "links added to LinkGrabber");
            self.inner
                .events
                .send(GrabberEvent::Added {
                    links: added.clone(),
                    source,
                })
                .ok();
        }
        for link in &added {
            self.spawn_probe(link.id, link.url.clone(), Some(link.package.clone()));
        }
        Ok(AddOutcome { added, duplicates })
    }

    pub async fn remove(&self, ids: &[LinkId]) -> Result<()> {
        let mut tx = self.inner.pool.begin().await?;
        for id in ids {
            sqlx::query("DELETE FROM link_grabber WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.removed(ids.to_vec());
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        let ids: Vec<LinkId> = sqlx::query_scalar("DELETE FROM link_grabber RETURNING id")
            .fetch_all(&self.inner.pool)
            .await?;
        self.removed(ids);
        Ok(())
    }

    /// Set the file name the download will be saved under (sanitised).
    pub async fn rename(&self, id: LinkId, name: &str) -> Result<()> {
        let name = filename::sanitize(name);
        sqlx::query("UPDATE link_grabber SET filename = ? WHERE id = ?")
            .bind(&name)
            .bind(id)
            .execute(&self.inner.pool)
            .await?;
        self.updated(id).await
    }

    /// Move links into the package called `name` (creating it if needed).
    pub async fn set_package(&self, ids: &[LinkId], name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::InvalidInput("Package name cannot be empty.".into()));
        }
        self.update_each(
            ids,
            "UPDATE link_grabber SET proposed_package = ? WHERE id = ?",
            name,
        )
        .await
    }

    /// Choose the folder for links; `None` restores the default folder.
    pub async fn set_dest_dir(&self, ids: &[LinkId], dir: Option<&Path>) -> Result<()> {
        let dir = dir.map(|d| d.to_string_lossy().into_owned());
        let pool = &self.inner.pool;
        for id in ids {
            sqlx::query("UPDATE link_grabber SET dest_dir = ? WHERE id = ?")
                .bind(dir.as_deref())
                .bind(id)
                .execute(pool)
                .await?;
            self.updated(*id).await?;
        }
        Ok(())
    }

    /// Choose the yt-dlp format for links; plain links are left alone. The
    /// size estimate follows the new choice.
    pub async fn set_format(&self, ids: &[LinkId], options: &YtDlpOptions) -> Result<()> {
        let json = serde_json::to_string(options)?;
        for id in ids {
            let Some(row) = self.row(*id).await? else {
                continue;
            };
            if row.kind() != DownloadKind::Ytdlp {
                continue;
            }
            let size = row
                .media_info()
                .and_then(|info| options.estimate_size(&info.formats));
            sqlx::query("UPDATE link_grabber SET handler_opts_json = ?, size = ? WHERE id = ?")
                .bind(&json)
                .bind(size.map(|s| s as i64))
                .bind(id)
                .execute(&self.inner.pool)
                .await?;
            self.updated(*id).await?;
        }
        Ok(())
    }

    /// Probe links again (after an error, or to refresh).
    pub async fn recheck(&self, ids: &[LinkId]) -> Result<()> {
        for id in ids {
            if let Some(row) = self.row(*id).await? {
                self.spawn_probe(row.id, row.source_url, None);
            }
        }
        Ok(())
    }

    /// Queue the given links as downloads, one package per (package name,
    /// folder), and remove them from the LinkGrabber. Links without a folder
    /// go to `default_dest`.
    pub async fn confirm(
        &self,
        ids: &[LinkId],
        default_dest: &Path,
        manager: &Manager,
    ) -> Result<Vec<PackageId>> {
        let wanted: HashSet<LinkId> = ids.iter().copied().collect();
        let links: Vec<GrabbedLink> = self
            .list()
            .await?
            .into_iter()
            .filter(|l| wanted.contains(&l.id))
            .collect();

        // Group by (package, folder), keeping the order packages first appear.
        let mut groups: Vec<((String, PathBuf), Vec<GrabbedLink>)> = Vec::new();
        for link in links {
            let key = (
                link.package.clone(),
                link.dest_dir
                    .clone()
                    .unwrap_or_else(|| default_dest.to_owned()),
            );
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, members)) => members.push(link),
                None => groups.push((key, vec![link])),
            }
        }

        let mut packages = Vec::with_capacity(groups.len());
        for ((name, dest_dir), members) in groups {
            let downloads = members.iter().filter_map(new_download).collect();
            let package = manager
                .add_package(NewPackage {
                    name,
                    dest_dir,
                    queue_id: DEFAULT_QUEUE,
                    downloads,
                })
                .await?;
            packages.push(package);
            let member_ids: Vec<LinkId> = members.iter().map(|l| l.id).collect();
            self.remove(&member_ids).await?;
        }
        Ok(packages)
    }

    async fn row(&self, id: LinkId) -> Result<Option<LinkRow>> {
        Ok(sqlx::query_as("SELECT * FROM link_grabber WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.inner.pool)
            .await?)
    }

    async fn update_each(&self, ids: &[LinkId], sql: &'static str, value: &str) -> Result<()> {
        for id in ids {
            sqlx::query(sql)
                .bind(value)
                .bind(id)
                .execute(&self.inner.pool)
                .await?;
            self.updated(*id).await?;
        }
        Ok(())
    }

    async fn updated(&self, id: LinkId) -> Result<()> {
        updated(&self.inner, id).await
    }

    fn removed(&self, ids: Vec<LinkId>) {
        if !ids.is_empty() {
            self.inner.events.send(GrabberEvent::Removed { ids }).ok();
        }
    }

    /// Probe in the background. `default_package` is the package name the link
    /// got when added; only that name may be replaced by an archive-set name,
    /// so a package the user picked is never overwritten.
    fn spawn_probe(&self, id: LinkId, url: String, default_package: Option<String>) {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            if let Err(e) = probe_link(&inner, id, &url, default_package.as_deref()).await {
                tracing::warn!(link = id, error = %e, "could not record link check");
            }
        });
    }
}

fn new_download(link: &GrabbedLink) -> Option<NewDownload> {
    let url = Url::parse(&link.url).ok()?;
    if link.kind != DownloadKind::Ytdlp {
        return Some(NewDownload {
            filename: link.filename.clone(),
            ..NewDownload::new(url)
        });
    }
    let title = link.media.as_ref().and_then(|m| m.title.clone());
    // The name only overrides yt-dlp's own `title [id]` when the user changed
    // it; otherwise two videos with the same title would collide.
    let renamed = link
        .filename
        .clone()
        .filter(|name| Some(name) != title.as_ref().map(|t| filename::sanitize(t)).as_ref());
    Some(NewDownload {
        url,
        filename: renamed,
        kind: DownloadKind::Ytdlp,
        ytdlp: Some(link.ytdlp.clone().unwrap_or_default()),
        title,
    })
}

async fn updated(inner: &Inner, id: LinkId) -> Result<()> {
    let row: Option<LinkRow> = sqlx::query_as("SELECT * FROM link_grabber WHERE id = ?")
        .bind(id)
        .fetch_optional(&inner.pool)
        .await?;
    // A row that's gone was removed meanwhile; nothing to report.
    if let Some(row) = row {
        inner
            .events
            .send(GrabberEvent::Updated {
                link: Box::new(row.into_link()),
            })
            .ok();
    }
    Ok(())
}

async fn probe_link(
    inner: &Inner,
    id: LinkId,
    url: &str,
    default_package: Option<&str>,
) -> Result<()> {
    let _permit = Arc::clone(&inner.probes)
        .acquire_owned()
        .await
        .expect("semaphore is never closed");
    let pool = &inner.pool;
    let marked =
        sqlx::query("UPDATE link_grabber SET availability = 'checking', error = NULL WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    if marked.rows_affected() == 0 {
        return Ok(()); // removed while waiting for a slot
    }
    updated(inner, id).await?;

    let parsed = Url::parse(url).map_err(|e| Error::InvalidUrl(e.to_string()));
    let result = match &parsed {
        Ok(parsed) => http::probe(&inner.client, &DownloadSpec::new(parsed.clone())).await,
        Err(e) => Err(Error::InvalidUrl(e.to_string())),
    };
    if let Ok(parsed) = &parsed
        && media::worth_asking(&result)
    {
        match media::resolve_link(inner, id, parsed, default_package).await {
            Ok(media::Verdict::Handled) => return updated(inner, id).await,
            Ok(media::Verdict::NotMedia) => {}
            Err(e) => tracing::warn!(link = id, error = %e, "yt-dlp check failed"),
        }
    }
    match result {
        Ok(probe) => {
            // COALESCE keeps a name the user typed while the probe ran.
            sqlx::query(
                "UPDATE link_grabber SET kind = 'http', info_json = NULL, availability = 'online',
                     error = NULL, filename = COALESCE(filename, ?), size = ?
                 WHERE id = ?",
            )
            .bind(&probe.filename)
            .bind(probe.size.map(|s| s as i64))
            .bind(id)
            .execute(pool)
            .await?;
            if let (Some(default), Some(set)) = (default_package, archive_set_name(&probe.filename))
            {
                sqlx::query(
                    "UPDATE link_grabber SET proposed_package = ? WHERE id = ? AND proposed_package = ?",
                )
                .bind(set)
                .bind(id)
                .bind(default)
                .execute(pool)
                .await?;
            }
        }
        Err(e) => {
            let availability = match &e {
                Error::LinkExpired(_)
                | Error::HttpStatus {
                    status: 404 | 410 | 451,
                    ..
                } => Availability::Offline,
                _ => Availability::Error,
            };
            tracing::debug!(link = id, error = %e, "link check failed");
            sqlx::query("UPDATE link_grabber SET availability = ?, error = ? WHERE id = ?")
                .bind(availability.as_str())
                .bind(e.to_string())
                .bind(id)
                .execute(pool)
                .await?;
        }
    }
    updated(inner, id).await
}
