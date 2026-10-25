//! Parsing gallery-dl's output: the `-j` message list, `--help`, and the
//! log lines it prints on stderr.
//!
//! `-j` prints one JSON array of messages once extraction ends. Each message
//! is an array whose first element is its type: `2` a directory (metadata
//! for the files after it), `3` a file URL with its metadata, `6` a URL
//! handed to another extractor (an album inside a profile, say). An error
//! ends the list as `[-1, {"error": ..., "message": ...}]`; older versions
//! wrote `["HttpError", "..."]`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

const DIRECTORY: i64 = 2;
const URL: i64 = 3;
const QUEUE: i64 = 6;

/// How many preview images are kept.
const THUMBNAILS: usize = 4;

/// What a gallery URL holds, as far as resolving found out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct Gallery {
    pub title: Option<String>,
    /// gallery-dl's extractor, e.g. `imgur`.
    pub category: Option<String>,
    /// What kind of page it is to that extractor, e.g. `album` or `user`.
    pub subcategory: Option<String>,
    /// Files found.
    pub files: u32,
    /// Further galleries the URL leads to (the albums of a profile). Their
    /// files are only counted once they download.
    pub sub_galleries: u32,
    /// Total size, when every file listed one.
    pub size: Option<u64>,
    /// First few image URLs, for a preview.
    pub thumbnails: Vec<String>,
    /// Resolving stopped early (too many files, or too slow), so `files` is
    /// a lower bound.
    pub partial: bool,
}

impl Gallery {
    /// Number of files a download will fetch, when resolving saw them all.
    pub fn known_total(&self) -> Option<u32> {
        (!self.partial && self.sub_galleries == 0).then_some(self.files)
    }
}

/// The `-j` output, split into what was found and the error that stopped it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub gallery: Gallery,
    pub error: Option<ToolError>,
}

/// An error gallery-dl reported, by exception class (`HttpError`,
/// `NotFoundError`, `AuthorizationError`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    pub kind: String,
    pub message: String,
}

impl ToolError {
    /// The item is gone or was never there (as opposed to a failure that
    /// might go away).
    pub fn is_gone(&self) -> bool {
        let lower = self.message.to_ascii_lowercase();
        self.kind == "NotFoundError"
            || lower.contains("404 not found")
            || lower.contains("410 gone")
            || lower.contains("not found")
            || lower.contains("does not exist")
            || lower.contains("deleted")
    }

    pub fn text(&self) -> String {
        if self.message.is_empty() {
            self.kind.clone()
        } else {
            format!("{}: {}", self.kind, self.message)
        }
    }
}

pub(super) fn resolved(json: &str, file_limit: Option<u32>) -> Result<Resolved> {
    let v: Value = serde_json::from_str(json.trim())
        .map_err(|e| Error::Tool(format!("could not read gallery-dl output: {e}")))?;
    let messages = v
        .as_array()
        .ok_or_else(|| Error::Tool("gallery-dl output is not a message list".into()))?;

    let mut gallery = Gallery::default();
    let mut error = None;
    let mut sizes = Some(0u64);
    for message in messages.iter().filter_map(Value::as_array) {
        let Some(kind) = message.first() else {
            continue;
        };
        if let Some(name) = kind.as_str() {
            error = Some(ToolError {
                kind: name.to_owned(),
                message: message.get(1).map(text).unwrap_or_default(),
            });
            continue;
        }
        match kind.as_i64() {
            Some(DIRECTORY) => {
                if let Some(meta) = message.get(1) {
                    note_metadata(&mut gallery, meta);
                }
            }
            Some(URL) => {
                gallery.files += 1;
                let meta = message.get(2);
                if let Some(meta) = meta {
                    // Some extractors only send file messages.
                    note_metadata(&mut gallery, meta);
                }
                sizes = match (sizes, meta.and_then(file_size)) {
                    (Some(total), Some(size)) => Some(total + size),
                    _ => None,
                };
                if let Some(url) = message.get(1).and_then(Value::as_str)
                    && gallery.thumbnails.len() < THUMBNAILS
                    && is_image(url, meta)
                {
                    gallery.thumbnails.push(url.to_owned());
                }
            }
            Some(QUEUE) => {
                gallery.sub_galleries += 1;
                if let Some(meta) = message.get(2) {
                    note_metadata(&mut gallery, meta);
                }
            }
            Some(n) if n < 0 => {
                let detail = message.get(1);
                error = Some(ToolError {
                    kind: detail
                        .and_then(|d| d.get("error"))
                        .and_then(Value::as_str)
                        .unwrap_or("Error")
                        .to_owned(),
                    message: detail
                        .and_then(|d| d.get("message"))
                        .map(text)
                        .unwrap_or_default(),
                });
            }
            _ => {}
        }
    }
    if gallery.files > 0 {
        gallery.size = sizes.filter(|&s| s > 0);
    }
    gallery.partial = file_limit.is_some_and(|limit| gallery.files >= limit);
    Ok(Resolved { gallery, error })
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Fill in title and extractor from the first metadata that has them.
fn note_metadata(gallery: &mut Gallery, meta: &Value) {
    let field = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_owned);
    if gallery.category.is_none() {
        gallery.category = field("category");
        gallery.subcategory = field("subcategory");
    }
    if gallery.title.is_none() {
        gallery.title = title_of(meta);
    }
}

/// Extractors name their galleries differently; these cover most of them.
/// A wiki `page` (Wikimedia's `Category:Big_Buck_Bunny`) comes first: there
/// `title` is the file's own.
fn title_of(meta: &Value) -> Option<String> {
    if let Some(page) = meta.get("page").and_then(Value::as_str) {
        return Some(page.replace('_', " ").trim().to_owned()).filter(|p| !p.is_empty());
    }
    let named = |v: &Value| -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            Value::Object(_) => ["title", "name", "nick", "username"]
                .iter()
                .find_map(|k| v.get(k).and_then(Value::as_str).map(str::to_owned)),
            _ => None,
        }
    };
    [
        "title",
        "gallery_title",
        "album_name",
        "album",
        "gallery",
        "set",
        "board",
        "manga",
        "search_tags",
        "user",
        "author",
        "artist",
        "username",
    ]
    .iter()
    .filter_map(|key| meta.get(key))
    .find_map(named)
    .map(|t| t.trim().to_owned())
    .filter(|t| !t.is_empty())
}

fn file_size(meta: &Value) -> Option<u64> {
    ["filesize", "size", "file_size"]
        .iter()
        .find_map(|k| meta.get(k).and_then(Value::as_u64))
        .filter(|&s| s > 0)
}

fn is_image(url: &str, meta: Option<&Value>) -> bool {
    const IMAGES: [&str; 7] = ["jpg", "jpeg", "png", "gif", "webp", "avif", "bmp"];
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return false; // `ytdl:` and `text:` pseudo-URLs
    }
    let ext = meta
        .and_then(|m| m.get("extension"))
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .or_else(|| {
            let path = url.split(['?', '#']).next()?;
            Some(path.rsplit_once('.')?.1.to_ascii_lowercase())
        });
    ext.is_some_and(|e| IMAGES.contains(&&*e))
}

/// Every option `--help` lists, short and long (`-D`, `--directory`).
pub fn help_flags(help: &str) -> HashSet<String> {
    let mut flags = HashSet::new();
    for line in help.lines() {
        let line = line.trim_start();
        if !line.starts_with('-') {
            continue;
        }
        // "-D, --directory PATH    Exact location ..." : options end at the
        // gap before the description.
        let spec = line.split("  ").next().unwrap_or(line);
        for part in spec.split(", ") {
            if let Some(flag) = part.split_whitespace().next()
                && flag.len() > 1
                && flag[1..]
                    .trim_start_matches('-')
                    .starts_with(|c: char| c.is_ascii_alphanumeric())
            {
                flags.insert(flag.to_owned());
            }
        }
    }
    flags
}

/// gallery-dl logs as `[extractor][level] message`. The last error is the
/// one that explains a failure; failing that, the last warning or line.
pub fn error_message<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> Option<String> {
    let lines: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    let tagged = |level: &str| {
        let marker = format!("][{level}] ");
        lines
            .iter()
            .rev()
            .find_map(|l| l.split_once(&marker).map(|(_, msg)| msg.trim().to_owned()))
    };
    match (tagged("error"), tagged("warning")) {
        // "Failed to download x.png" alone hides the reason, which the
        // downloader logged as a warning just before it:
        // "'429 Too many requests' for 'https://...'".
        (Some(error), Some(warning)) if error.starts_with("Failed to download") => {
            let cause = warning.split(" for '").next().unwrap_or(&warning);
            Some(format!("{error}: {}", cause.trim_matches('\'')))
        }
        (Some(error), _) => Some(error),
        (None, Some(warning)) => Some(warning),
        (None, None) => lines.last().map(|l| (*l).to_owned()),
    }
}

/// gallery-dl has no extractor for the URL.
pub fn is_unsupported(message: &str) -> bool {
    message.contains("Unsupported URL") || message.contains("No suitable extractor")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `gallery-dl -j` on an album: version, directory, two files.
    const ALBUM: &str = r#"[
[1, 1],
[2, {"category": "imgur", "subcategory": "album", "album": {"id": "abc", "title": "Mountain trip"}, "count": 2}],
[3, "https://i.imgur.com/one.jpg", {"category": "imgur", "subcategory": "album", "filename": "one", "extension": "jpg", "size": 1000, "num": 1}],
[3, "https://i.imgur.com/two.mp4", {"category": "imgur", "subcategory": "album", "filename": "two", "extension": "mp4", "size": 3000, "num": 2}]
]"#;

    #[test]
    fn reads_an_album() {
        let r = resolved(ALBUM, None).unwrap();
        let g = r.gallery;
        assert_eq!(g.title.as_deref(), Some("Mountain trip"));
        assert_eq!(g.category.as_deref(), Some("imgur"));
        assert_eq!(g.subcategory.as_deref(), Some("album"));
        assert_eq!(g.files, 2);
        assert_eq!(g.size, Some(4000));
        assert_eq!(g.thumbnails, ["https://i.imgur.com/one.jpg"]);
        assert_eq!(g.known_total(), Some(2));
        assert!(r.error.is_none());
    }

    #[test]
    fn a_profile_counts_its_albums() {
        let json = r#"[
[6, "https://www.imgur.com/a/x", {"category": "imgur", "subcategory": "user", "user": {"username": "someone"}}],
[6, "https://www.imgur.com/a/y", {"category": "imgur", "subcategory": "user"}]
]"#;
        let g = resolved(json, None).unwrap().gallery;
        assert_eq!(g.sub_galleries, 2);
        assert_eq!(g.files, 0);
        assert_eq!(g.title.as_deref(), Some("someone"));
        assert_eq!(g.known_total(), None);
        assert_eq!(g.size, None);
    }

    #[test]
    fn unknown_sizes_leave_the_total_unknown() {
        let json = r#"[
[3, "https://x.test/a.png", {"filesize": 10}],
[3, "https://x.test/b.png?w=1", {}]
]"#;
        let g = resolved(json, None).unwrap().gallery;
        assert_eq!(g.size, None);
        assert_eq!(g.thumbnails.len(), 2, "extension from the URL path");
    }

    #[test]
    fn hitting_the_limit_marks_the_count_partial() {
        let json = r#"[[3, "https://x.test/a.jpg", {}], [3, "https://x.test/b.jpg", {}]]"#;
        let g = resolved(json, Some(2)).unwrap().gallery;
        assert!(g.partial);
        assert_eq!(g.known_total(), None);
    }

    #[test]
    fn reads_both_error_shapes() {
        let new = r#"[[-1, {"error": "NotFoundError", "message": "Requested gallery could not be found"}]]"#;
        let e = resolved(new, None).unwrap().error.unwrap();
        assert_eq!(e.kind, "NotFoundError");
        assert!(e.is_gone());
        assert_eq!(
            e.text(),
            "NotFoundError: Requested gallery could not be found"
        );

        let old = r#"[[2, {"title": "T"}], ["HttpError", "'503 Service Unavailable' for 'https://x.test/'"]]"#;
        let r = resolved(old, None).unwrap();
        assert_eq!(r.gallery.title.as_deref(), Some("T"));
        let e = r.error.unwrap();
        assert_eq!(e.kind, "HttpError");
        assert!(!e.is_gone());
    }

    #[test]
    fn rejects_non_lists() {
        assert!(resolved("{}", None).is_err());
        assert!(resolved("not json", None).is_err());
    }

    #[test]
    fn finds_flags_in_help() {
        let help = "\
usage: gallery-dl [OPTION]... URL...

General Options:
  -h, --help                  Print this help message and exit
  -d, --destination PATH      Target location for file downloads
  -D, --directory PATH        Exact location for file downloads
  --range RANGE               Index range(s) specifying which files to
                              download. These can be either a constant value,
                              range, or slice (e.g. '5', '8-20', or '1:24:3')
  -o, --option KEY=VALUE      Additional options. Example: -o browser=firefox
  --cookies-from-browser BROWSER[/DOMAIN][+KEYRING][:PROFILE][::CONTAINER]
";
        let flags = help_flags(help);
        for f in [
            "-h",
            "--help",
            "-D",
            "--directory",
            "--range",
            "-o",
            "--option",
            "--cookies-from-browser",
        ] {
            assert!(flags.contains(f), "{f}");
        }
        assert!(!flags.contains("--child-range"));
        assert!(!flags.iter().any(|f| f.contains("RANGE")));
    }

    #[test]
    fn picks_the_last_error_line() {
        let stderr = "\
[imgur][warning] Slow down
[gallery-dl][error] Unsupported URL 'https://example.com/'
";
        let msg = error_message(stderr.lines()).unwrap();
        assert_eq!(msg, "Unsupported URL 'https://example.com/'");
        assert!(is_unsupported(&msg));
        assert_eq!(
            error_message("[x][warning] only a warning\n".lines()).as_deref(),
            Some("only a warning")
        );
        assert_eq!(error_message("plain\n".lines()).as_deref(), Some("plain"));
        assert_eq!(error_message("".lines()), None);
    }

    /// Recorded from gallery-dl 1.32.17 hitting Wikimedia's rate limit.
    #[test]
    fn failed_downloads_say_why() {
        let stderr = "\
[downloader.http][warning] '429 Too many requests (8d818c2)' for 'https://upload.wikimedia.org/wikipedia/commons/4/47/3D-Window_BBB_Blender_3.png?utm_source=commons.wikimedia.org'
[download][error] Failed to download 3D-Window BBB Blender 3 (540cc186).png
";
        assert_eq!(
            error_message(stderr.lines()).as_deref(),
            Some(
                "Failed to download 3D-Window BBB Blender 3 (540cc186).png: 429 Too many requests (8d818c2)"
            )
        );
    }

    /// Recorded from gallery-dl 1.32.17 (`-j --range 1-2` on a Wikimedia
    /// Commons category, metadata trimmed). A directory message comes before
    /// every file, with that file's `title`; `page` names the category.
    const WIKIMEDIA: &str = r#"[
  [2, {"category": "wikimediacommons", "count": 1, "imagerepository": "local", "lang": "", "ns": 6,
       "page": "Category:Big_Buck_Bunny", "pageid": 51994980, "subcategory": "category",
       "title": "File:-youtube-dl https www.youtube.com watch v=YE7VzlLtp-4.png"}],
  [3, "https://upload.wikimedia.org/wikipedia/commons/f/fb/-youtube-dl_https_www.youtube.com_watch_v%3DYE7VzlLtp-4.png?utm_source=commons.wikimedia.org&utm_campaign=imageinfo&utm_content=original",
      {"category": "wikimediacommons", "extension": "png", "filename": "-youtube-dl https www.youtube.com watch v=YE7VzlLtp-4",
       "size": 12251, "width": 663, "height": 140}],
  [2, {"category": "wikimediacommons", "count": 1, "imagerepository": "local", "lang": "", "ns": 6,
       "page": "Category:Big_Buck_Bunny", "pageid": 22420553, "subcategory": "category",
       "title": "File:3D-Window BBB Blender 2.jpg"}],
  [3, "https://upload.wikimedia.org/wikipedia/commons/d/d1/3D-Window_BBB_Blender_2.jpg?utm_source=commons.wikimedia.org&utm_campaign=imageinfo&utm_content=original",
      {"category": "wikimediacommons", "extension": "jpg", "filename": "3D-Window BBB Blender 2",
       "size": 572473, "width": 1280, "height": 956}]
]"#;

    #[test]
    fn reads_real_wikimedia_output() {
        let g = resolved(WIKIMEDIA, Some(2000)).unwrap().gallery;
        assert_eq!(g.title.as_deref(), Some("Category:Big Buck Bunny"));
        assert_eq!(g.category.as_deref(), Some("wikimediacommons"));
        assert_eq!(g.subcategory.as_deref(), Some("category"));
        assert_eq!(g.files, 2);
        assert_eq!(g.size, Some(12251 + 572473));
        assert_eq!(g.thumbnails.len(), 2);
        assert!(!g.partial);
        // A category that doesn't exist: an empty list, exit status 0.
        assert_eq!(resolved("[]", None).unwrap().gallery.files, 0);
    }
}
