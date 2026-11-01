//! The yt-dlp side of link checking: deciding when to ask the tools at all,
//! and turning yt-dlp's answer into LinkGrabber rows (one per video;
//! playlists expand).

use std::collections::HashSet;

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{Availability, GrabberEvent, Inner, LinkId, LinkRow, LinkSource};
use crate::http::{ProbeResult, filename};
use crate::manager::DownloadKind;
use crate::proxy::ProxyChoice;
use crate::request::RequestContext;
use crate::tools::ToolName;

use crate::ytdlp::{self, Media, MediaInfo, YtDlpOptions};
use crate::{Error, Result};

/// What the LinkGrabber shows about a video.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct MediaSummary {
    pub title: Option<String>,
    pub uploader: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub extractor: Option<String>,
    /// Video heights on offer, highest first; empty for audio-only or flat
    /// playlist entries (formats are only known once downloaded).
    pub heights: Vec<u32>,
    pub is_live: bool,
}

impl MediaSummary {
    pub(super) fn from_info(info: &MediaInfo) -> Self {
        let mut heights: Vec<u32> = info
            .formats
            .iter()
            .filter(|f| f.has_video())
            .filter_map(|f| f.height)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        heights.sort_unstable_by(|a, b| b.cmp(a));
        Self {
            title: info.title.clone(),
            uploader: info.uploader.clone(),
            duration: info.duration,
            thumbnail: info.thumbnail.clone(),
            extractor: info.extractor.clone(),
            heights,
            is_live: info.is_live,
        }
    }
}

/// A plain probe that found a web page (or failed) may be a page yt-dlp or
/// gallery-dl understands. A file served directly never is.
pub(super) fn worth_asking(probe: &Result<ProbeResult>) -> bool {
    match probe {
        Ok(p) => p.content_type.as_deref().is_some_and(|t| {
            let t = t.to_ascii_lowercase();
            t.starts_with("text/html") || t.starts_with("application/xhtml")
        }),
        Err(Error::InvalidUrl(_)) => false,
        Err(_) => true,
    }
}

/// A tool's verdict on a link.
pub(super) enum Verdict {
    /// Recorded as the tool's link (or links).
    Handled,
    /// Not a site the tool knows.
    NotMedia,
    /// The tool knows the site but couldn't read this page. Not recorded:
    /// another tool may still manage.
    Failed(String),
}

/// What yt-dlp made of a link, before anything is recorded.
pub(super) enum Lookup {
    NotMedia,
    Failed(String),
    Found(Media),
}

/// Ask yt-dlp about `url`. Installs yt-dlp on first use.
pub(super) async fn lookup(inner: &Inner, url: &Url, request: &RequestContext) -> Result<Lookup> {
    let Some(tools) = &inner.tools else {
        return Ok(Lookup::NotMedia);
    };
    let exe = match tools.ensure(ToolName::Ytdlp).await {
        Ok(exe) => exe,
        Err(e) => {
            tracing::warn!(error = %e, "yt-dlp unavailable; checking links as plain files");
            return Ok(Lookup::NotMedia);
        }
    };
    let proxy = inner.network.tool_proxy(url, ProxyChoice::Auto);
    let cancel = CancellationToken::new();
    match ytdlp::resolve(&exe, url, request, proxy.as_deref(), &cancel).await {
        Ok(media) => Ok(Lookup::Found(media)),
        Err(Error::Tool(message)) if is_unsupported(&message) => Ok(Lookup::NotMedia),
        Err(Error::Tool(message)) => Ok(Lookup::Failed(message)),
        Err(e) => Err(e),
    }
}

/// yt-dlp's catch-all extractors pick up any media embedded in any page
/// (the one video on a Wikimedia category of 80 files, say), so a site
/// extractor in gallery-dl should win over them.
pub(super) fn is_generic(media: &Media) -> bool {
    media.extractor().is_some_and(|e| {
        let e = e.to_ascii_lowercase();
        e == "generic" || e == "html5mediaembed"
    })
}

/// Record what yt-dlp found: a video, or one link per playlist entry.
pub(super) async fn record(
    inner: &Inner,
    id: LinkId,
    media: Media,
    default_package: Option<&str>,
) -> Result<Verdict> {
    match media {
        Media::Video(info) => record_video(inner, id, &info, default_package).await?,
        Media::Playlist { title, entries, .. } if entries.is_empty() => {
            let name = title.unwrap_or_else(|| "The playlist".into());
            return Ok(Verdict::Failed(format!("{name} is empty")));
        }
        Media::Playlist { title, entries, .. } => {
            expand_playlist(inner, id, title, entries, default_package).await?
        }
    }
    Ok(Verdict::Handled)
}

fn is_unsupported(message: &str) -> bool {
    message.contains("Unsupported URL") || message.contains("No video formats found")
}

/// Record that `kind`'s tool knows the site but couldn't read the link.
pub(super) async fn record_failure(
    inner: &Inner,
    id: LinkId,
    kind: DownloadKind,
    message: &str,
) -> Result<()> {
    let lower = message.to_ascii_lowercase();
    let gone = [
        "unavailable",
        "private video",
        "has been removed",
        "does not exist",
        "not found",
        "404",
        "410 gone",
        "deleted",
    ]
    .iter()
    .any(|w| lower.contains(w));
    let availability = if gone {
        Availability::Offline
    } else {
        Availability::Error
    };
    sqlx::query("UPDATE link_grabber SET kind = ?, availability = ?, error = ? WHERE id = ?")
        .bind(kind.as_str())
        .bind(availability.as_str())
        .bind(message)
        .bind(id)
        .execute(&inner.pool)
        .await?;
    Ok(())
}

async fn record_video(
    inner: &Inner,
    id: LinkId,
    info: &MediaInfo,
    default_package: Option<&str>,
) -> Result<()> {
    let options: YtDlpOptions = current_options(inner, id).await?;
    let title = info.title.as_deref().map(filename::sanitize);
    // COALESCE keeps a name the user typed while yt-dlp ran.
    sqlx::query(
        "UPDATE link_grabber SET kind = 'ytdlp', availability = 'online', error = NULL,
             filename = COALESCE(filename, ?), size = ?, info_json = ?, handler_opts_json = ?
         WHERE id = ?",
    )
    .bind(title.as_deref())
    .bind(options.estimate_size(&info.formats).map(|s| s as i64))
    .bind(serde_json::to_string(info)?)
    .bind(serde_json::to_string(&options)?)
    .bind(id)
    .execute(&inner.pool)
    .await?;
    if let (Some(default), Some(title)) = (default_package, &info.title) {
        rename_default_package(inner, id, default, title).await?;
    }
    Ok(())
}

async fn expand_playlist(
    inner: &Inner,
    id: LinkId,
    title: Option<String>,
    entries: Vec<ytdlp::PlaylistEntry>,
    default_package: Option<&str>,
) -> Result<()> {
    let Some(original) = row(inner, id).await? else {
        return Ok(()); // removed meanwhile
    };
    // A package the user picked wins over the playlist's title.
    let package = match (&original.proposed_package, default_package, &title) {
        (Some(current), Some(default), Some(title)) if current == default => title.clone(),
        (Some(current), _, _) => current.clone(),
        (None, _, Some(title)) => title.clone(),
        (None, _, None) => "Playlist".into(),
    };
    let options = original
        .handler_opts_json
        .clone()
        .unwrap_or(serde_json::to_string(&YtDlpOptions::default())?);

    let pool = &inner.pool;
    let existing: HashSet<String> = sqlx::query_scalar("SELECT source_url FROM link_grabber")
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    let mut added = Vec::new();
    let mut tx = pool.begin().await?;
    for entry in entries {
        let Ok(url) = Url::parse(&entry.url) else {
            continue;
        };
        if existing.contains(url.as_str()) || !seen.insert(url.to_string()) {
            continue;
        }
        // Some sites (SoundCloud) list entries without titles; the URL slug is
        // a stand-in until yt-dlp names the file at download time.
        let title = entry.title.clone().or_else(|| filename::from_url(&url));
        let info = MediaInfo {
            id: None,
            title: title.clone(),
            uploader: None,
            duration: entry.duration,
            thumbnail: None,
            extractor: None,
            webpage_url: Some(url.to_string()),
            is_live: false,
            formats: Vec::new(),
        };
        let link: LinkRow = sqlx::query_as(
            "INSERT INTO link_grabber
                 (source_url, kind, filename, availability, proposed_package, dest_dir, handler_opts_json,
                  info_json, request_json)
             VALUES (?, 'ytdlp', ?, 'online', ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(url.as_str())
        .bind(title.as_deref().map(filename::sanitize))
        .bind(&package)
        .bind(original.dest_dir.as_deref())
        .bind(&options)
        .bind(serde_json::to_string(&info)?)
        // Entries are on the same site and need the same cookies.
        .bind(original.request_json.as_deref())
        .fetch_one(&mut *tx)
        .await?;
        added.push(link.into_link());
    }
    sqlx::query("DELETE FROM link_grabber WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let added = super::with_history(inner, added).await?;

    tracing::info!(entries = added.len(), %package, "playlist expanded");

    inner
        .events
        .send(GrabberEvent::Removed { ids: vec![id] })
        .ok();
    if !added.is_empty() {
        inner
            .events
            .send(GrabberEvent::Added {
                links: added,
                source: LinkSource::Playlist,
            })
            .ok();
    }
    Ok(())
}

/// Only the name the link was given when added may be replaced.
pub(super) async fn rename_default_package(
    inner: &Inner,
    id: LinkId,
    default: &str,
    to: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE link_grabber SET proposed_package = ? WHERE id = ? AND proposed_package = ?",
    )
    .bind(to)
    .bind(id)
    .bind(default)
    .execute(&inner.pool)
    .await?;
    Ok(())
}

async fn row(inner: &Inner, id: LinkId) -> Result<Option<LinkRow>> {
    Ok(sqlx::query_as("SELECT * FROM link_grabber WHERE id = ?")
        .bind(id)
        .fetch_optional(&inner.pool)
        .await?)
}

/// The link's stored handler options, if they read as `T`; else `T`'s defaults.
pub(super) async fn current_options<T: serde::de::DeserializeOwned + Default>(
    inner: &Inner,
    id: LinkId,
) -> Result<T> {
    Ok(row(inner, id)
        .await?
        .and_then(|r| r.handler_opts_json)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(content_type: Option<&str>) -> Result<ProbeResult> {
        Ok(ProbeResult {
            final_url: Url::parse("https://example.com/").unwrap(),
            size: None,
            accepts_ranges: false,
            etag: None,
            last_modified: None,
            content_type: content_type.map(str::to_owned),
            filename: "x".into(),
        })
    }

    #[test]
    fn asks_ytdlp_about_pages_and_failures_only() {
        assert!(worth_asking(&probe(Some("text/html; charset=utf-8"))));
        assert!(!worth_asking(&probe(Some("video/mp4"))));
        assert!(!worth_asking(&probe(None)));
        assert!(worth_asking(&Err(Error::LinkExpired(403))));
        assert!(!worth_asking(&Err(Error::InvalidUrl("x".into()))));
    }

    #[test]
    fn summary_lists_distinct_heights() {
        let fmt = |h: Option<u32>, v: &str| ytdlp::FormatInfo {
            format_id: "f".into(),
            ext: "mp4".into(),
            height: h,
            vcodec: Some(v.into()),
            acodec: None,
            size: None,
            tbr: None,
            note: None,
        };
        let info = MediaInfo {
            id: None,
            title: Some("T".into()),
            uploader: None,
            duration: None,
            thumbnail: None,
            extractor: None,
            webpage_url: None,
            is_live: false,
            formats: vec![
                fmt(Some(720), "avc1"),
                fmt(Some(1080), "vp9"),
                fmt(Some(720), "vp9"),
                fmt(None, "none"),
            ],
        };
        assert_eq!(MediaSummary::from_info(&info).heights, [1080, 720]);
    }
}
