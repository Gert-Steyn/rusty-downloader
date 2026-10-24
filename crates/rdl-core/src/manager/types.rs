use std::path::PathBuf;

use serde::Serialize;
use url::Url;

use crate::ytdlp::YtDlpOptions;

pub type PackageId = i64;
pub type DownloadId = i64;
pub type QueueId = i64;

/// The "Main" queue seeded by the initial migration.
pub const DEFAULT_QUEUE: QueueId = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Active,
    Paused,
    Completed,
    Failed,
    Canceled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::Active => "active",
            Status::Paused => "paused",
            Status::Completed => "completed",
            Status::Failed => "failed",
            Status::Canceled => "canceled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Status::Queued,
            "active" => Status::Active,
            "paused" => Status::Paused,
            "completed" => Status::Completed,
            "failed" => Status::Failed,
            "canceled" => Status::Canceled,
            _ => return None,
        })
    }
}

/// Which handler runs a download. Stored as the `downloads.kind` text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum DownloadKind {
    #[default]
    Http,
    Ytdlp,
}

impl DownloadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadKind::Http => "http",
            DownloadKind::Ytdlp => "ytdlp",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewDownload {
    pub url: Url,
    /// Overrides the server-provided name. Sanitised on insert. For yt-dlp
    /// this is the name without extension (yt-dlp picks the extension).
    pub filename: Option<String>,
    pub kind: DownloadKind,
    /// Format choice for `Ytdlp` downloads.
    pub ytdlp: Option<YtDlpOptions>,
    /// Media title, shown until the file name is known.
    pub title: Option<String>,
}

impl NewDownload {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            filename: None,
            kind: DownloadKind::Http,
            ytdlp: None,
            title: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewPackage {
    pub name: String,
    pub dest_dir: PathBuf,
    pub queue_id: QueueId,
    pub downloads: Vec<NewDownload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub id: DownloadId,
    pub package_id: PackageId,
    pub kind: String,
    pub source_url: String,
    pub filename: Option<String>,
    pub dest_path: Option<PathBuf>,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub status: Status,
    pub error: Option<String>,
    /// Format choice, for yt-dlp downloads.
    pub ytdlp: Option<YtDlpOptions>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub id: PackageId,
    pub name: String,
    pub dest_dir: PathBuf,
    pub queue_id: QueueId,
    pub priority: i64,
    pub downloads: Vec<Download>,
}

/// Everything the UI needs to stay in sync without polling. Delivered on a
/// broadcast channel: a receiver that lags simply misses events and should
/// re-read the full list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    PackageAdded {
        package_id: PackageId,
    },
    PackageRemoved {
        package_id: PackageId,
    },
    DownloadRemoved {
        download_id: DownloadId,
        package_id: PackageId,
    },
    StatusChanged {
        download_id: DownloadId,
        status: Status,
        error: Option<String>,
    },
    /// Name, size and destination are known after the first probe.
    Resolved {
        download_id: DownloadId,
        filename: String,
        size: Option<u64>,
    },
    Progress {
        download_id: DownloadId,
        downloaded: u64,
        total: Option<u64>,
        speed_bps: u64,
        connections: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_text() {
        for s in [
            Status::Queued,
            Status::Active,
            Status::Paused,
            Status::Completed,
            Status::Failed,
            Status::Canceled,
        ] {
            assert_eq!(Status::parse(s.as_str()), Some(s));
        }
        assert_eq!(Status::parse("bogus"), None);
    }
}
