//! Picking a local filename: `Content-Disposition` (RFC 6266 / RFC 5987), the URL
//! path as a fallback, then sanitising for Windows.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use url::Url;

pub const FALLBACK_NAME: &str = "download";
const MAX_NAME_CHARS: usize = 200;
const MAX_EXT_CHARS: usize = 16;

/// Extract the filename from a `Content-Disposition` header value. `filename*`
/// wins over `filename` when both are present, as RFC 6266 requires.
pub fn from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    // The first part is the disposition type ("attachment", "inline").
    for param in split_params(value).into_iter().skip(1) {
        let Some((key, val)) = param.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                if let Some(name) = decode_ext_value(val.trim()) {
                    return Some(name).filter(|n| !n.is_empty());
                }
            }
            "filename" => plain = Some(unquote(val.trim())),
            _ => {}
        }
    }
    plain.filter(|n| !n.is_empty())
}

/// Last non-empty path segment of the URL, percent-decoded.
pub fn from_url(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_decode_str(segment).decode_utf8_lossy().into_owned();
    Some(decoded).filter(|n| !n.is_empty())
}

/// Make `name` safe to create on Windows: no reserved characters or device
/// names, no trailing dots/spaces, and a bounded length that keeps the extension.
pub fn sanitize(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let mut name = replaced.trim().trim_end_matches(['.', ' ']).to_owned();
    if name.is_empty() {
        return FALLBACK_NAME.to_owned();
    }

    let stem = name.split('.').next().unwrap_or_default().trim_end();
    if is_reserved_device_name(stem) {
        name.insert(0, '_');
    }
    truncate_keeping_extension(&name)
}

/// `dir/name`, or `dir/stem (n).ext` with the first free `n` if that exists.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    (1..)
        .map(|n| match ext {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        })
        .find(|p| !p.exists())
        .expect("unbounded range always finds a free name")
}

/// Split header parameters on `;`, ignoring semicolons inside quoted strings.
fn split_params(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut in_quotes, mut escaped) = (0, false, false);
    for (i, c) in value.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                parts.push(value[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts
}

fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            chars.next().unwrap_or(c)
        } else {
            c
        });
    }
    out
}

/// RFC 5987 `charset'language'percent-encoded-value`.
fn decode_ext_value(value: &str) -> Option<String> {
    let mut parts = value.splitn(3, '\'');
    let charset = parts.next()?.to_ascii_lowercase();
    let _language = parts.next()?;
    let encoded = parts.next()?;
    let bytes: Vec<u8> = percent_decode_str(encoded).collect();
    match charset.as_str() {
        "utf-8" => String::from_utf8(bytes).ok(),
        // Latin-1 maps each byte straight to the same code point.
        "iso-8859-1" => Some(bytes.into_iter().map(char::from).collect()),
        _ => None,
    }
}

fn is_reserved_device_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            let (prefix, digit) = upper.split_at(upper.len().min(3));
            matches!(prefix, "COM" | "LPT")
                && digit.len() == 1
                && digit.as_bytes()[0].is_ascii_digit()
        }
    }
}

fn truncate_keeping_extension(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_CHARS {
        return name.to_owned();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.chars().count() <= MAX_EXT_CHARS => {
            let keep = MAX_NAME_CHARS - ext.chars().count() - 1;
            let stem: String = stem.chars().take(keep).collect();
            format!("{}.{ext}", stem.trim_end_matches(['.', ' ']))
        }
        _ => name.chars().take(MAX_NAME_CHARS).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_plain_and_quoted() {
        assert_eq!(
            from_content_disposition("attachment; filename=report.pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            from_content_disposition(r#"attachment; filename="my \"best\" file; v2.zip""#)
                .as_deref(),
            Some(r#"my "best" file; v2.zip"#)
        );
    }

    #[test]
    fn content_disposition_prefers_extended_value() {
        let header =
            r#"attachment; filename="fallback.txt"; filename*=UTF-8''na%C3%AFve%20file.txt"#;
        assert_eq!(
            from_content_disposition(header).as_deref(),
            Some("naïve file.txt")
        );
        assert_eq!(
            from_content_disposition("attachment; filename*=iso-8859-1'en'%E9t%E9.txt").as_deref(),
            Some("été.txt")
        );
    }

    #[test]
    fn content_disposition_without_filename() {
        assert_eq!(from_content_disposition("inline"), None);
        assert_eq!(from_content_disposition(r#"attachment; filename="""#), None);
    }

    #[test]
    fn url_fallback_decodes_last_segment() {
        let url = Url::parse("https://example.com/files/My%20Video.mp4?token=abc").unwrap();
        assert_eq!(from_url(&url).as_deref(), Some("My Video.mp4"));
        let dir = Url::parse("https://example.com/files/").unwrap();
        assert_eq!(from_url(&dir).as_deref(), Some("files"));
        assert_eq!(from_url(&Url::parse("https://example.com/").unwrap()), None);
    }

    #[test]
    fn sanitize_replaces_reserved_characters() {
        assert_eq!(
            sanitize(r#"a<b>c:d"e/f\g|h?i*j.txt"#),
            "a_b_c_d_e_f_g_h_i_j.txt"
        );
        assert_eq!(sanitize("tab\there.txt"), "tab_here.txt");
        assert_eq!(sanitize("  trailing dots... "), "trailing dots");
        assert_eq!(sanitize("..."), FALLBACK_NAME);
    }

    #[test]
    fn sanitize_escapes_device_names() {
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("nul.txt"), "_nul.txt");
        assert_eq!(sanitize("COM1.log"), "_COM1.log");
        assert_eq!(sanitize("COMPUTER.txt"), "COMPUTER.txt");
        assert_eq!(sanitize("console.txt"), "console.txt");
    }

    #[test]
    fn sanitize_truncates_but_keeps_extension() {
        let long = format!("{}.mkv", "x".repeat(300));
        let out = sanitize(&long);
        assert_eq!(out.chars().count(), MAX_NAME_CHARS);
        assert!(out.ends_with(".mkv"));
    }

    #[test]
    fn unique_path_numbers_collisions() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.zip"), dir.path().join("a.zip"));

        std::fs::write(dir.path().join("a.zip"), "").unwrap();
        std::fs::write(dir.path().join("a (1).zip"), "").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.zip"),
            dir.path().join("a (2).zip")
        );

        std::fs::write(dir.path().join("README"), "").unwrap();
        assert_eq!(
            unique_path(dir.path(), "README"),
            dir.path().join("README (1)")
        );
    }
}
