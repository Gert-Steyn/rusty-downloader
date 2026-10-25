//! Finding a tool's newest release and its checksum, on GitHub or on
//! Codeberg (whose release API returns the same fields).

use reqwest::Url;
use reqwest::header::ACCEPT;
use serde::Deserialize;

use super::{Forge, Upstream};
use crate::{Error, Result};

/// The asset to install, as published right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Latest {
    /// Human-readable release name: the tag, or the asset's upload date for
    /// rolling tags like FFmpeg-Builds' `latest`.
    pub release: String,
    pub url: Url,
    pub size: Option<u64>,
    /// Lowercase hex SHA-256 from the release's checksum file (or the asset's
    /// digest).
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
    updated_at: Option<String>,
    /// `sha256:<hex>`, computed by GitHub on upload.
    digest: Option<String>,
}

pub(super) async fn latest(client: &reqwest::Client, api: &Url, up: &Upstream) -> Result<Latest> {
    let host = match up.forge {
        Forge::GitHub => "GitHub",
        Forge::Codeberg => "Codeberg",
    };
    let url = api
        .join(&format!("repos/{}/releases/latest", up.repo))
        .map_err(|e| Error::InvalidUrl(e.to_string()))?;
    let response = client
        .get(url.clone())
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(Error::Tool(match status {
            403 | 429 => format!(
                "{host} refused the update check for {} (rate limited); try again later",
                up.repo
            ),
            _ => format!("{host} returned HTTP {status} for {}", up.repo),
        }));
    }
    let release: Release = serde_json::from_str(&response.text().await?)?;

    let find = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| {
                Error::Tool(format!(
                    "release {} of {} has no {name}",
                    release.tag_name, up.repo
                ))
            })
    };
    let asset = find(up.asset)?;
    let sha256 = match up.sums {
        Some(sums_name) => {
            let sums_url = parse_url(&find(sums_name)?.browser_download_url)?;
            let response = client.get(sums_url.clone()).send().await?;
            crate::http::check_status(response.status(), &sums_url)?;
            let sums = response.text().await?;
            checksum_for(&sums, up.asset).ok_or_else(|| {
                Error::Tool(format!(
                    "{sums_name} does not list a checksum for {}",
                    up.asset
                ))
            })?
        }
        None => asset
            .digest
            .as_deref()
            .and_then(sha256_digest)
            .ok_or_else(|| {
                Error::Tool(format!(
                    "{host} lists no SHA-256 digest for {} in {}",
                    up.asset, up.repo
                ))
            })?,
    };

    let release_name = match (&*release.tag_name, &asset.updated_at) {
        ("latest", Some(at)) => at.get(..10).unwrap_or(at).to_owned(),
        (tag, _) => tag.to_owned(),
    };
    Ok(Latest {
        release: release_name,
        url: parse_url(&asset.browser_download_url)?,
        size: asset.size,
        sha256,
    })
}

fn parse_url(raw: &str) -> Result<Url> {
    Url::parse(raw).map_err(|e| Error::InvalidUrl(format!("{raw}: {e}")))
}

/// Look up `file` in `sha256sum`-style output (`<hex>  <name>`, or `*<name>`
/// for binary mode).
pub(super) fn checksum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == file && is_sha256_hex(hash)).then(|| hash.to_ascii_lowercase())
    })
}

/// The hex part of a GitHub asset digest (`sha256:<hex>`).
fn sha256_digest(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?;
    is_sha256_hex(hex).then(|| hex.to_ascii_lowercase())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_checksums_in_both_formats() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  yt-dlp\n{b} *yt-dlp.exe\nnot a line\n");
        assert_eq!(checksum_for(&sums, "yt-dlp.exe"), Some("b".repeat(64)));
        assert_eq!(checksum_for(&sums, "yt-dlp"), Some(a));
        assert_eq!(checksum_for(&sums, "yt-dlp_x86.exe"), None);
        assert_eq!(checksum_for("short  yt-dlp.exe", "yt-dlp.exe"), None);
    }

    #[test]
    fn reads_asset_digests() {
        let hex = "5B".repeat(32);
        assert_eq!(
            sha256_digest(&format!("sha256:{hex}")),
            Some("5b".repeat(32))
        );
        assert_eq!(sha256_digest(&format!("sha512:{hex}")), None);
        assert_eq!(sha256_digest("sha256:abc"), None);
    }
}
