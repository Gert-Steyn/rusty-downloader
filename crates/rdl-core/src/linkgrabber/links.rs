//! Pure helpers: finding links in text and proposing package names.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use url::Url;

use crate::http::filename;

static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s"'<>]+"#).expect("valid regex"));

/// Multipart archive names, each capturing the shared base name.
static ARCHIVE_PART: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)^(.+)\.part\d+\.rar$",          // name.part01.rar
        r"(?i)^(.+)\.(?:7z|zip|rar)\.\d{3}$", // name.7z.001
        r"(?i)^(.+)\.r\d{2}$",                // name.r00
        r"(?i)^(.+)\.z\d{2}$",                // name.z01
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid regex"))
    .collect()
});

/// Distinct http(s) URLs in `text`, in order, without trailing punctuation
/// picked up from prose (`see https://x.com/a.zip.`).
pub fn extract_links(text: &str) -> Vec<Url> {
    let mut seen = HashSet::new();
    LINK.find_iter(text)
        .filter_map(|m| {
            let raw = m
                .as_str()
                .trim_end_matches([')', ',', '.', ';', ':', '!', '?', ']', '}']);
            Url::parse(raw).ok()
        })
        .filter(|url| seen.insert(url.as_str().to_owned()))
        .collect()
}

/// The base name shared by all parts of a split archive, if `name` is one.
pub fn archive_set_name(name: &str) -> Option<String> {
    ARCHIVE_PART
        .iter()
        .find_map(|re| re.captures(name))
        .map(|c| c[1].to_owned())
}

/// Package name for links added together: one link is named after its file,
/// several after their host.
pub fn default_package_name(urls: &[Url]) -> String {
    if let [url] = urls
        && let Some(name) = filename::from_url(url)
    {
        return match name.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
            _ => name,
        };
    }
    urls.first()
        .and_then(|u| u.host_str())
        .map(|h| h.trim_start_matches("www.").to_owned())
        .unwrap_or_else(|| "Links".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links_from_prose() {
        let text = "Grab https://example.com/a.zip, then (https://cdn.example.org/b.iso).\n\
                    Again: https://example.com/a.zip and ftp://nope.example/c plus HTTP://Example.com/D";
        let links: Vec<String> = extract_links(text).iter().map(|u| u.to_string()).collect();
        assert_eq!(
            links,
            [
                "https://example.com/a.zip",
                "https://cdn.example.org/b.iso",
                "http://example.com/D"
            ]
        );
    }

    #[test]
    fn extracts_nothing_from_plain_text() {
        assert!(extract_links("no links here, just http and https words").is_empty());
    }

    #[test]
    fn recognises_split_archives() {
        assert_eq!(
            archive_set_name("Movie.2026.part01.rar").as_deref(),
            Some("Movie.2026")
        );
        assert_eq!(archive_set_name("backup.7z.003").as_deref(), Some("backup"));
        assert_eq!(archive_set_name("data.r00").as_deref(), Some("data"));
        assert_eq!(archive_set_name("data.z01").as_deref(), Some("data"));
        assert_eq!(archive_set_name("data.rar"), None);
        assert_eq!(archive_set_name("song.mp3"), None);
    }

    #[test]
    fn names_packages_after_file_or_host() {
        let one = [Url::parse("https://example.com/files/Big%20Video.mp4").unwrap()];
        assert_eq!(default_package_name(&one), "Big Video");
        let many = [
            Url::parse("https://www.example.com/a.zip").unwrap(),
            Url::parse("https://www.example.com/b.zip").unwrap(),
        ];
        assert_eq!(default_package_name(&many), "example.com");
    }
}
