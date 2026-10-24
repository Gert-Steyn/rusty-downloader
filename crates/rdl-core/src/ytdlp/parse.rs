//! Parsing yt-dlp's JSON output. Fields are optional throughout: extractors
//! differ wildly in what they fill in.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

/// What a URL resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum Media {
    Video(MediaInfo),
    Playlist {
        title: Option<String>,
        entries: Vec<PlaylistEntry>,
    },
}

/// One video (or audio track), with its formats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub id: Option<String>,
    pub title: Option<String>,
    pub uploader: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    /// yt-dlp's extractor, e.g. `Youtube`.
    pub extractor: Option<String>,
    pub webpage_url: Option<String>,
    pub is_live: bool,
    pub formats: Vec<FormatInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatInfo {
    pub format_id: String,
    pub ext: String,
    pub height: Option<u32>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    /// Exact or approximate size in bytes.
    pub size: Option<u64>,
    /// Total bitrate in kbit/s.
    pub tbr: Option<f64>,
    pub note: Option<String>,
}

impl FormatInfo {
    fn codec(c: &Option<String>) -> bool {
        c.as_deref().is_some_and(|c| c != "none")
    }

    pub fn has_video(&self) -> bool {
        Self::codec(&self.vcodec) || (self.vcodec.is_none() && self.height.is_some())
    }

    pub fn has_audio(&self) -> bool {
        Self::codec(&self.acodec)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistEntry {
    pub url: String,
    pub title: Option<String>,
    pub duration: Option<f64>,
}

pub(super) fn media(json: &str) -> Result<Media> {
    let v: Value = serde_json::from_str(json.trim())
        .map_err(|e| Error::Tool(format!("could not read yt-dlp output: {e}")))?;
    if v.get("_type").and_then(Value::as_str) == Some("playlist") {
        let entries = v
            .get("entries")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(entry).collect())
            .unwrap_or_default();
        return Ok(Media::Playlist {
            title: string(&v, "title"),
            entries,
        });
    }
    Ok(Media::Video(info(&v)))
}

fn info(v: &Value) -> MediaInfo {
    let formats = v
        .get("formats")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(format).collect())
        .unwrap_or_default();
    MediaInfo {
        id: string(v, "id"),
        title: string(v, "title"),
        uploader: string(v, "uploader").or_else(|| string(v, "channel")),
        duration: v.get("duration").and_then(Value::as_f64),
        thumbnail: string(v, "thumbnail"),
        extractor: string(v, "extractor_key").or_else(|| string(v, "extractor")),
        webpage_url: string(v, "webpage_url"),
        is_live: v.get("is_live").and_then(Value::as_bool).unwrap_or(false),
        formats,
    }
}

fn format(v: &Value) -> Option<FormatInfo> {
    // Storyboards are images, not media.
    if string(v, "format_note").is_some_and(|n| n == "storyboard")
        || string(v, "ext").is_some_and(|e| e == "mhtml")
    {
        return None;
    }
    Some(FormatInfo {
        format_id: string(v, "format_id")?,
        ext: string(v, "ext").unwrap_or_default(),
        height: v.get("height").and_then(Value::as_u64).map(|h| h as u32),
        vcodec: string(v, "vcodec"),
        acodec: string(v, "acodec"),
        size: number(v, "filesize").or_else(|| number(v, "filesize_approx")),
        tbr: v.get("tbr").and_then(Value::as_f64),
        note: string(v, "format_note"),
    })
}

fn entry(v: &Value) -> Option<PlaylistEntry> {
    let url = string(v, "url").or_else(|| string(v, "webpage_url"))?;
    Some(PlaylistEntry {
        url,
        title: string(v, "title"),
        duration: v.get("duration").and_then(Value::as_f64),
    })
}

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Sizes come as integers or floats depending on the extractor.
fn number(v: &Value, key: &str) -> Option<u64> {
    let n = v.get(key)?;
    n.as_u64()
        .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
}

/// One `RDLPROG` line.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub(super) struct Progress {
    #[serde(default)]
    pub status: String,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub total_bytes_estimate: Option<f64>,
    pub speed: Option<f64>,
    pub filename: Option<String>,
}

pub(super) fn progress(json: &str) -> Option<Progress> {
    // yt-dlp prints `NA` for missing fields in some versions; such lines are skipped.
    serde_json::from_str(json.trim()).ok()
}

/// One `RDLINFO` line.
pub(super) struct Planned {
    pub filename: String,
    pub size: Option<u64>,
}

pub(super) fn planned(json: &str) -> Option<Planned> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    Some(Planned {
        filename: string(&v, "filename")?,
        size: number(&v, "filesize").or_else(|| number(&v, "filesize_approx")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_video() {
        let json = r#"{"id":"abc","title":"Clip","uploader":"Someone","duration":61.5,
            "extractor_key":"Youtube","webpage_url":"https://www.youtube.com/watch?v=abc",
            "formats":[
              {"format_id":"sb0","ext":"mhtml","format_note":"storyboard"},
              {"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a.40.2","filesize":3000,"tbr":129.5},
              {"format_id":"137","ext":"mp4","height":1080,"vcodec":"avc1","acodec":"none","filesize_approx":5.0e4}
            ]}"#;
        let Media::Video(info) = media(json).unwrap() else {
            panic!("expected a video");
        };
        assert_eq!(info.title.as_deref(), Some("Clip"));
        assert_eq!(info.extractor.as_deref(), Some("Youtube"));
        assert_eq!(info.formats.len(), 2);
        assert!(info.formats[0].has_audio() && !info.formats[0].has_video());
        assert_eq!(info.formats[1].size, Some(50_000));
        assert!(info.formats[1].has_video());
    }

    #[test]
    fn parses_a_flat_playlist() {
        let json = r#"{"_type":"playlist","title":"Mix","entries":[
            {"_type":"url","url":"https://www.youtube.com/watch?v=a","title":"A","duration":10},
            {"_type":"url","title":"no url"},
            {"_type":"url","url":"https://www.youtube.com/watch?v=b"}]}"#;
        let Media::Playlist { title, entries } = media(json).unwrap() else {
            panic!("expected a playlist");
        };
        assert_eq!(title.as_deref(), Some("Mix"));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title.as_deref(), Some("A"));
        assert_eq!(entries[1].title, None);
    }

    #[test]
    fn rejects_non_json() {
        assert!(media("ERROR: nope").is_err());
    }

    #[test]
    fn parses_progress_and_planned_lines() {
        let p = progress(r#"{"status":"downloading","downloaded_bytes":1024,"total_bytes":null,"total_bytes_estimate":4096.7,"speed":512.25,"filename":"C:\\dl\\a.f137.mp4"}"#).unwrap();
        assert_eq!(p.downloaded_bytes, Some(1024));
        assert_eq!(p.total_bytes_estimate, Some(4096.7));
        assert_eq!(p.filename.as_deref(), Some(r"C:\dl\a.f137.mp4"));
        assert!(progress("NA").is_none());

        let info =
            planned(r#"{"filename":"C:\\dl\\a.mp4","filesize":null,"filesize_approx":1234}"#)
                .unwrap();
        assert_eq!(info.filename, r"C:\dl\a.mp4");
        assert_eq!(info.size, Some(1234));
    }
}
