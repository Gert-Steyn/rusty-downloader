//! Pure helpers: finding links in text and proposing package names.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use url::Url;

use crate::http::filename;

/// Web, FTP and SFTP links, and `magnet:?` links (which have no `//`). A magnet
/// link runs on through its `&` parameters and their `%` escapes.
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:(?:https?|s?ftps?)://|magnet:\?)[^\s"'<>]+"#).expect("valid regex")
});

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

/// Distinct http(s), ftp(s), sftp and magnet URLs in `text`, in order, without
/// trailing punctuation picked up from prose (`see https://x.com/a.zip.`).
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

/// The first value of query parameter `key` in a magnet link, decoded; empty values are absent.
fn magnet_param(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, v)| k == key && !v.is_empty())
        .map(|(_, v)| v.into_owned())
}

/// The info hash from a magnet link's `xt=urn:btih:` parameter, as written
/// (40 hex digits or 32 base32 characters).
pub fn magnet_info_hash(url: &Url) -> Option<String> {
    url.query_pairs().find_map(|(k, v)| {
        (k == "xt")
            .then(|| {
                v.strip_prefix("urn:btih:")
                    .filter(|h| !h.is_empty())
                    .map(str::to_owned)
            })
            .flatten()
    })
}

/// The name a magnet link proposes: its `dn` (display name) sanitised for
/// Windows, else its info hash.
pub fn magnet_name(url: &Url) -> Option<String> {
    magnet_param(url, "dn")
        .filter(|name| !name.trim().is_empty())
        .map(|name| filename::sanitize(&name))
        .or_else(|| magnet_info_hash(url))
}

/// The size a magnet link declares with its `xl` (exact length) parameter.
pub fn magnet_size(url: &Url) -> Option<u64> {
    magnet_param(url, "xl")?.parse().ok()
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
    // A magnet has no path to name it by; its `dn` (or info hash) does.
    if let [url] = urls
        && url.scheme() == "magnet"
        && let Some(name) = magnet_name(url)
    {
        return name;
    }
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
                    Again: https://example.com/a.zip and ftp://nope.example/c, sftp://me@nope.example/d plus HTTP://Example.com/D";
        let links: Vec<String> = extract_links(text).iter().map(|u| u.to_string()).collect();
        assert_eq!(
            links,
            [
                "https://example.com/a.zip",
                "https://cdn.example.org/b.iso",
                "ftp://nope.example/c",
                "sftp://me@nope.example/d",
                "http://example.com/D"
            ]
        );
    }

    #[test]
    fn extracts_magnet_links_whole() {
        let text = "Here: magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Big%20Buck%20Bunny&tr=udp%3A%2F%2Ftracker.example%3A80/announce, then \"magnet:?xt=urn:btih:ABC\" <MAGNET:?xt=urn:btih:DEF>.";
        let links: Vec<String> = extract_links(text).iter().map(|u| u.to_string()).collect();
        assert_eq!(
            links,
            [
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Big%20Buck%20Bunny&tr=udp%3A%2F%2Ftracker.example%3A80/announce",
                "magnet:?xt=urn:btih:ABC",
                "magnet:?xt=urn:btih:DEF",
            ]
        );
    }

    #[test]
    fn magnet_names_come_from_dn_then_info_hash() {
        let named = Url::parse(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Big+Buck%20Bunny%3A%20Test%3F&xl=12345",
        )
        .unwrap();
        assert_eq!(
            magnet_name(&named).as_deref(),
            Some("Big Buck Bunny_ Test_")
        );
        assert_eq!(magnet_size(&named), Some(12345));

        let blank_dn = Url::parse("magnet:?dn=%20&xt=urn:btih:ABC").unwrap();
        assert_eq!(magnet_name(&blank_dn).as_deref(), Some("ABC"));
        assert_eq!(magnet_info_hash(&blank_dn).as_deref(), Some("ABC"));
        assert_eq!(magnet_size(&blank_dn), None);

        let bare = Url::parse("magnet:?xt=urn:sha1:NOTATORRENT").unwrap();
        assert_eq!(magnet_info_hash(&bare), None);
        assert_eq!(magnet_name(&bare), None);
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
        let magnet = [Url::parse(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Big+Buck+Bunny",
        )
        .unwrap()];
        assert_eq!(default_package_name(&magnet), "Big Buck Bunny");
    }
}
