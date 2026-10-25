//! What a browser would send along with a link: the page it came from, its
//! user agent, and its cookies for the site. The browser extension supplies
//! these; they are stored with the link (`request_json`) and used for every
//! request the download makes.
//!
//! Cookies are matched to each URL the way a browser does (RFC 6265: domain,
//! path, `Secure`, expiry), so a redirect to another host or a CDN gets only
//! the cookies meant for it. yt-dlp and gallery-dl get them as a temporary
//! Netscape `cookies.txt` ([`CookieFile`]) that is deleted after the run.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use url::{Host, Url};

use crate::Result;

/// Prefix of temporary cookie files, so stale ones can be found again.
const COOKIE_FILE_PREFIX: &str = "rdl-cookies-";

/// Referer, user agent and cookies for a link. Empty means "send nothing
/// extra", which is what every link without a browser behind it gets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RequestContext {
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub cookies: Vec<Cookie>,
}

/// One cookie, shaped like the browser's `chrome.cookies.Cookie`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// `example.com` or `.example.com`; the leading dot means nothing on
    /// its own (see `host_only`).
    pub domain: String,
    /// Only for exactly `domain`, not its subdomains (a cookie set without
    /// a `Domain` attribute).
    #[serde(default)]
    pub host_only: bool,
    #[serde(default = "root_path")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    /// Unix seconds; `None` is a session cookie.
    #[serde(default)]
    pub expires: Option<i64>,
}

fn root_path() -> String {
    "/".into()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Cookie {
    /// The domain without its leading dot, lowercased.
    fn bare_domain(&self) -> String {
        self.domain
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase()
    }

    /// Whether this cookie can be written as a header and a `cookies.txt`
    /// line: a name, a domain, and no characters that would break either.
    fn is_valid(&self) -> bool {
        let clean = |s: &str| !s.chars().any(|c| c.is_control() || c == ';');
        !self.name.is_empty()
            && !self.name.contains(['=', ' '])
            && clean(&self.name)
            && clean(&self.value)
            && !self.bare_domain().is_empty()
            && !self.domain.trim().contains(char::is_whitespace)
            && clean(&self.path)
    }

    fn is_expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|e| e <= now)
    }

    /// Whether a browser would send this cookie to `url` (RFC 6265 §5.4).
    pub fn matches(&self, url: &Url, now: i64) -> bool {
        if self.is_expired(now) || (self.secure && url.scheme() != "https") {
            return false;
        }
        let Some(host) = url.host() else { return false };
        let domain = self.bare_domain();
        let host_matches = match host {
            Host::Domain(name) => {
                let name = name.to_ascii_lowercase();
                name == domain
                    || (!self.host_only
                        && name
                            .strip_suffix(&domain)
                            .is_some_and(|rest| rest.ends_with('.')))
            }
            // IP addresses only match exactly.
            ip => ip.to_string() == domain,
        };
        host_matches && path_matches(url.path(), &self.path)
    }

    /// The `cookies.txt` line: domain, subdomains, path, secure, expiry
    /// (empty for a session cookie), name, value.
    fn netscape_line(&self) -> String {
        let domain = self.bare_domain();
        let (domain, subdomains) = if self.host_only {
            (domain, "FALSE")
        } else {
            (format!(".{domain}"), "TRUE")
        };
        let flag = |b: bool| if b { "TRUE" } else { "FALSE" };
        format!(
            "{}{domain}\t{subdomains}\t{}\t{}\t{}\t{}\t{}\n",
            if self.http_only { "#HttpOnly_" } else { "" },
            self.path,
            flag(self.secure),
            self.expires.map(|e| e.to_string()).unwrap_or_default(),
            self.name,
            self.value,
        )
    }
}

/// RFC 6265 §5.1.4: `/docs` matches `/docs`, `/docs/` and `/docs/a`, not `/docsx`.
fn path_matches(request: &str, cookie: &str) -> bool {
    let request = if request.is_empty() { "/" } else { request };
    match request.strip_prefix(cookie) {
        Some(rest) => rest.is_empty() || cookie.ends_with('/') || rest.starts_with('/'),
        None => false,
    }
}

impl RequestContext {
    pub fn is_empty(&self) -> bool {
        self.referer.is_none() && self.user_agent.is_none() && self.cookies.is_empty()
    }

    /// Drop whatever can't be sent: a referer that isn't an http(s) URL, a
    /// user agent that isn't a valid header, malformed or expired cookies.
    /// Everything from outside (the browser, the database) goes through this.
    pub fn cleaned(mut self) -> Self {
        self.referer = self
            .referer
            .map(|r| r.trim().to_owned())
            .filter(|r| Url::parse(r).is_ok_and(|u| matches!(u.scheme(), "http" | "https")));
        self.user_agent = self
            .user_agent
            .map(|ua| ua.trim().to_owned())
            .filter(|ua| !ua.is_empty() && HeaderValue::from_str(ua).is_ok());
        let now = now_secs();
        self.cookies.retain(|c| c.is_valid() && !c.is_expired(now));
        for cookie in &mut self.cookies {
            cookie.domain = cookie.domain.trim().to_ascii_lowercase();
            if !cookie.path.starts_with('/') {
                cookie.path = root_path();
            }
        }
        self
    }

    /// For storing: `None` when there's nothing to store.
    pub fn to_json(&self) -> Result<Option<String>> {
        Ok(if self.is_empty() {
            None
        } else {
            Some(serde_json::to_string(self)?)
        })
    }

    /// From storage; unreadable JSON counts as empty.
    pub fn from_json(json: Option<&str>) -> Self {
        json.and_then(|j| serde_json::from_str::<Self>(j).ok())
            .map(Self::cleaned)
            .unwrap_or_default()
    }

    /// The `Cookie` header for `url`: matching cookies, longer paths first.
    pub fn cookie_header(&self, url: &Url) -> Option<String> {
        let now = now_secs();
        let mut matching: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| c.matches(url, now))
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        Some(
            matching
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    /// Headers to send with a request for `url`.
    pub fn headers_for(&self, url: &Url) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let mut put = |name, value: &str| {
            if let Ok(value) = HeaderValue::from_str(value) {
                headers.insert(name, value);
            }
        };
        if let Some(referer) = &self.referer {
            put(REFERER, referer);
        }
        if let Some(ua) = &self.user_agent {
            put(USER_AGENT, ua);
        }
        if let Some(cookies) = self.cookie_header(url) {
            put(COOKIE, &cookies);
        }
        headers
    }

    /// All cookies as a Netscape `cookies.txt`.
    pub fn netscape(&self) -> String {
        let mut text = String::from("# Netscape HTTP Cookie File\n");
        for cookie in &self.cookies {
            text.push_str(&cookie.netscape_line());
        }
        text
    }
}

/// A temporary `cookies.txt` for one tool run, deleted when dropped. Lives
/// in the user's temp folder, which only they (and administrators) can read.
#[derive(Debug)]
pub struct CookieFile {
    path: PathBuf,
}

impl CookieFile {
    /// Write the cookies out, or `None` if there are none.
    pub fn create(request: &RequestContext) -> Result<Option<Self>> {
        if request.cookies.is_empty() {
            return Ok(None);
        }
        let dir = std::env::temp_dir();
        loop {
            let path = dir.join(format!(
                "{COOKIE_FILE_PREFIX}{}-{:016x}.txt",
                std::process::id(),
                fastrand::u64(..)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    // From here on, dropping `this` removes the file.
                    let this = Self { path };
                    file.write_all(request.netscape().as_bytes())?;
                    return Ok(Some(this));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CookieFile {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), error = %e, "could not delete cookie file");
        }
    }
}

/// Delete cookie files left behind by a crash. Only old ones: another
/// process (a test run, say) may be using fresh ones.
pub fn sweep_cookie_files() {
    const STALE: Duration = Duration::from_secs(24 * 60 * 60);
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(COOKIE_FILE_PREFIX) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: format!("{name}-value"),
            domain: domain.into(),
            host_only: false,
            path: "/".into(),
            secure: false,
            http_only: false,
            expires: None,
        }
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn domains_match_like_a_browser() {
        let now = now_secs();
        let shared = cookie("a", ".example.com");
        assert!(shared.matches(&url("https://example.com/f"), now));
        assert!(shared.matches(&url("https://cdn.Example.com/f"), now));
        assert!(!shared.matches(&url("https://badexample.com/f"), now));
        assert!(!shared.matches(&url("https://example.org/f"), now));

        let host_only = Cookie {
            host_only: true,
            ..cookie("b", "example.com")
        };
        assert!(host_only.matches(&url("https://example.com/"), now));
        assert!(!host_only.matches(&url("https://cdn.example.com/"), now));

        let ip = cookie("c", "127.0.0.1");
        assert!(ip.matches(&url("http://127.0.0.1:8080/x"), now));
        assert!(!ip.matches(&url("http://localhost/x"), now));
    }

    #[test]
    fn paths_secure_and_expiry_are_honoured() {
        let now = now_secs();
        let docs = Cookie {
            path: "/docs".into(),
            ..cookie("p", "example.com")
        };
        assert!(docs.matches(&url("https://example.com/docs"), now));
        assert!(docs.matches(&url("https://example.com/docs/a.pdf"), now));
        assert!(!docs.matches(&url("https://example.com/docsx"), now));
        assert!(!docs.matches(&url("https://example.com/"), now));

        let secure = Cookie {
            secure: true,
            ..cookie("s", "example.com")
        };
        assert!(secure.matches(&url("https://example.com/"), now));
        assert!(!secure.matches(&url("http://example.com/"), now));

        let expired = Cookie {
            expires: Some(now - 10),
            ..cookie("e", "example.com")
        };
        assert!(!expired.matches(&url("https://example.com/"), now));
    }

    #[test]
    fn cookie_header_puts_longer_paths_first() {
        let request = RequestContext {
            cookies: vec![
                cookie("root", "example.com"),
                Cookie {
                    path: "/a/b".into(),
                    ..cookie("deep", "example.com")
                },
                cookie("other", "example.org"),
            ],
            ..Default::default()
        };
        assert_eq!(
            request
                .cookie_header(&url("https://example.com/a/b/c"))
                .as_deref(),
            Some("deep=deep-value; root=root-value")
        );
        assert_eq!(request.cookie_header(&url("https://example.net/")), None);
    }

    #[test]
    fn cleaning_drops_what_cannot_be_sent() {
        let request = RequestContext {
            referer: Some("javascript:alert(1)".into()),
            user_agent: Some("  Mozilla/5.0 \n".into()),
            cookies: vec![
                cookie("ok", " .Example.COM "),
                cookie("", "example.com"),
                cookie("a=b", "example.com"),
                Cookie {
                    value: "x\ty".into(),
                    ..cookie("tab", "example.com")
                },
                cookie("nodomain", "."),
                Cookie {
                    expires: Some(1),
                    ..cookie("old", "example.com")
                },
                Cookie {
                    path: "".into(),
                    ..cookie("nopath", "example.com")
                },
            ],
        }
        .cleaned();
        assert_eq!(request.referer, None);
        assert_eq!(request.user_agent.as_deref(), Some("Mozilla/5.0"));
        let names: Vec<&str> = request.cookies.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["ok", "nopath"]);
        assert_eq!(request.cookies[0].domain, ".example.com");
        assert_eq!(request.cookies[1].path, "/");
    }

    #[test]
    fn headers_carry_referer_agent_and_cookies() {
        let request = RequestContext {
            referer: Some("https://example.com/page".into()),
            user_agent: Some("TestBrowser/1".into()),
            cookies: vec![cookie("sid", "example.com")],
        };
        let headers = request.headers_for(&url("https://example.com/file"));
        assert_eq!(headers[REFERER], "https://example.com/page");
        assert_eq!(headers[USER_AGENT], "TestBrowser/1");
        assert_eq!(headers[COOKIE], "sid=sid-value");
        let elsewhere = request.headers_for(&url("https://other.test/file"));
        assert!(elsewhere.get(COOKIE).is_none());
        assert!(
            RequestContext::default()
                .headers_for(&url("https://x.test/"))
                .is_empty()
        );
    }

    #[test]
    fn json_round_trip_and_empty_is_null() {
        assert_eq!(RequestContext::default().to_json().unwrap(), None);
        let request = RequestContext {
            cookies: vec![cookie("sid", "example.com")],
            ..Default::default()
        };
        let json = request.to_json().unwrap();
        assert_eq!(RequestContext::from_json(json.as_deref()), request);
        assert_eq!(
            RequestContext::from_json(Some("not json")),
            RequestContext::default()
        );
        // The browser's own field names, with optional ones left out.
        let browser: Cookie =
            serde_json::from_str(r#"{"name":"n","value":"v","domain":"x.test","hostOnly":true}"#)
                .unwrap();
        assert!(browser.host_only);
        assert_eq!(browser.path, "/");
    }

    #[test]
    fn netscape_file_is_written_and_removed() {
        let request = RequestContext {
            cookies: vec![
                Cookie {
                    secure: true,
                    http_only: true,
                    expires: Some(2_000_000_000),
                    ..cookie("sid", ".example.com")
                },
                Cookie {
                    host_only: true,
                    path: "/a".into(),
                    ..cookie("pref", "example.com")
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            request.netscape(),
            "# Netscape HTTP Cookie File\n\
             #HttpOnly_.example.com\tTRUE\t/\tTRUE\t2000000000\tsid\tsid-value\n\
             example.com\tFALSE\t/a\tFALSE\t\tpref\tpref-value\n"
        );

        assert!(
            CookieFile::create(&RequestContext::default())
                .unwrap()
                .is_none()
        );
        let file = CookieFile::create(&request).unwrap().unwrap();
        let path = file.path().to_owned();
        assert_eq!(fs::read_to_string(&path).unwrap(), request.netscape());
        drop(file);
        assert!(!path.exists());
    }
}
