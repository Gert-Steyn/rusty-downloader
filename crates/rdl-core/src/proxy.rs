//! Proxy profiles and the rules that pick one per site. Profiles live in
//! `proxy_profiles` (their passwords in [`crate::secrets`] under
//! `proxy/<id>`), rules in `proxy_rules` (a NULL profile means "connect
//! directly"), and the default under the `proxy.default` setting.
//! [`crate::network::Network`] turns them into connections.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};

use crate::db::Pool;
use crate::{Error, Result, settings};

/// Setting holding the default profile's id (absent: connect directly).
const DEFAULT_KEY: &str = "proxy.default";
/// More rules than anyone types by hand; keeps a bad import from stalling routing.
pub const MAX_RULES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Http,
    Https,
    Socks5,
}

impl ProxyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProxyKind::Http => "http",
            ProxyKind::Https => "https",
            ProxyKind::Socks5 => "socks5",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "http" => ProxyKind::Http,
            "https" => ProxyKind::Https,
            "socks5" => ProxyKind::Socks5,
            _ => return None,
        })
    }

    /// URL scheme. SOCKS5 resolves names at the proxy (`socks5h`), so
    /// lookups don't leak around it.
    fn scheme(self) -> &'static str {
        match self {
            ProxyKind::Http => "http",
            ProxyKind::Https => "https",
            ProxyKind::Socks5 => "socks5h",
        }
    }
}

/// A proxy as the UI sees it: never the password, only whether there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProxyProfile {
    pub id: i64,
    pub name: String,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub has_password: bool,
}

/// A new or changed profile.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProxyInput {
    pub name: String,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    /// `None` keeps the stored password; an empty one removes it.
    pub password: Option<String>,
}

impl ProxyInput {
    /// Trimmed and checked; the error says what to fix.
    pub fn validated(mut self) -> Result<Self> {
        self.name = self.name.trim().to_owned();
        self.host = self.host.trim().to_owned();
        self.username = self
            .username
            .map(|u| u.trim().to_owned())
            .filter(|u| !u.is_empty());
        let invalid = |m: &str| Err(Error::InvalidInput(m.to_owned()));
        if self.name.is_empty() || self.name.chars().count() > 60 {
            return invalid("Give the proxy a name of 1 to 60 characters.");
        }
        if self.host.is_empty()
            || self.host.contains("://")
            || self
                .host
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '?' | '#'))
        {
            return invalid(
                "Enter the proxy's host name or IP address only, without http:// or a port.",
            );
        }
        // A colon is only allowed in an IPv6 address; elsewhere it's a port.
        if self.host.contains(':')
            && self
                .host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::Ipv6Addr>()
                .is_err()
        {
            return invalid("Enter the port in its own field, not after the host.");
        }
        if self.port == 0 {
            return invalid("Enter a port from 1 to 65535.");
        }
        if self.username.as_deref().is_some_and(|u| u.contains(':')) {
            return invalid("A user name can't contain \":\".");
        }
        Ok(self)
    }
}

/// Which connection a download uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum ProxyChoice {
    /// The rules decide, then the default.
    #[default]
    Auto,
    Direct,
    Profile {
        id: i64,
    },
}

impl ProxyChoice {
    /// Stored as `downloads.proxy`: NULL, `direct` or the profile's id.
    pub fn to_db(self) -> Option<String> {
        match self {
            ProxyChoice::Auto => None,
            ProxyChoice::Direct => Some("direct".into()),
            ProxyChoice::Profile { id } => Some(id.to_string()),
        }
    }

    /// Anything unreadable counts as automatic.
    pub fn from_db(value: Option<&str>) -> Self {
        match value {
            Some("direct") => ProxyChoice::Direct,
            Some(id) => id
                .parse()
                .map(|id| ProxyChoice::Profile { id })
                .unwrap_or_default(),
            None => ProxyChoice::Auto,
        }
    }
}

/// Sites matching `pattern` connect through `profile` (`None`: directly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProxyRule {
    pub pattern: String,
    pub profile: Option<i64>,
}

/// The rules in order (first match wins) and the profile for everything
/// else (`None`: connect directly).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProxySettings {
    pub default: Option<i64>,
    pub rules: Vec<ProxyRule>,
}

impl ProxySettings {
    /// Patterns trimmed and lowercased; every profile must exist.
    pub fn validated(mut self, profiles: &[i64]) -> Result<Self> {
        if self.rules.len() > MAX_RULES {
            return Err(Error::InvalidInput(format!("At most {MAX_RULES} rules.")));
        }
        for rule in &mut self.rules {
            rule.pattern = rule
                .pattern
                .trim()
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .to_ascii_lowercase();
            if rule.pattern.is_empty() || rule.pattern.contains(char::is_whitespace) {
                return Err(Error::InvalidInput(
                    "Every rule needs a site, like example.com or *.example.com.".into(),
                ));
            }
        }
        let known = |id: &Option<i64>| id.is_none_or(|id| profiles.contains(&id));
        if !known(&self.default) || !self.rules.iter().all(|r| known(&r.profile)) {
            return Err(Error::InvalidInput(
                "That proxy doesn't exist any more.".into(),
            ));
        }
        Ok(self)
    }

    /// The profile for `host` (`None`: directly): the first matching rule,
    /// else the default.
    pub fn profile_for(&self, host: &str) -> Option<i64> {
        self.rules
            .iter()
            .find(|r| host_matches(&r.pattern, host))
            .map_or(self.default, |r| r.profile)
    }
}

/// `example.com` matches that host and its subdomains; with `*` the
/// whole host must match (`*.example.com`, `cdn*.example.net`). Case is ignored.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    if pattern.contains('*') {
        return settings::wildcard_match(&pattern, &host);
    }
    host == pattern
        || host
            .strip_suffix(&pattern)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// Characters escaped in a URL's user name and password.
const USERINFO: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The proxy as a URL with its login, as reqwest, yt-dlp and gallery-dl take it.
pub fn proxy_url(profile: &ProxyProfile, password: Option<&str>) -> String {
    let login = match (&profile.username, password) {
        (Some(user), Some(pass)) => format!(
            "{}:{}@",
            utf8_percent_encode(user, USERINFO),
            utf8_percent_encode(pass, USERINFO)
        ),
        (Some(user), None) => format!("{}@", utf8_percent_encode(user, USERINFO)),
        (None, _) => String::new(),
    };
    let host = if profile.host.contains(':') && !profile.host.starts_with('[') {
        format!("[{}]", profile.host)
    } else {
        profile.host.clone()
    };
    format!("{}://{login}{host}:{}", profile.kind.scheme(), profile.port)
}

/// Where a profile's password is kept in [`crate::secrets`].
pub fn secret_key(id: i64) -> String {
    format!("proxy/{id}")
}

#[derive(sqlx::FromRow)]
struct ProfileRow {
    id: i64,
    name: String,
    kind: String,
    host: String,
    port: i64,
    username: Option<String>,
    keyring_ref: Option<String>,
}

pub(crate) async fn profiles(pool: &Pool) -> Result<Vec<ProxyProfile>> {
    let rows: Vec<ProfileRow> = sqlx::query_as(
        "SELECT id, name, kind, host, port, username, keyring_ref FROM proxy_profiles ORDER BY name COLLATE NOCASE, id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ProxyProfile {
            id: r.id,
            name: r.name,
            kind: ProxyKind::parse(&r.kind).unwrap_or(ProxyKind::Http),
            host: r.host,
            port: r.port as u16,
            username: r.username,
            has_password: r.keyring_ref.is_some(),
        })
        .collect())
}

/// A clash with another profile's name, as a message for the user.
fn name_taken(e: sqlx::Error, name: &str) -> Error {
    if e.as_database_error()
        .is_some_and(|d| d.is_unique_violation())
    {
        Error::InvalidInput(format!("There is already a proxy called \"{name}\"."))
    } else {
        e.into()
    }
}

pub(crate) async fn insert_profile(pool: &Pool, p: &ProxyInput) -> Result<i64> {
    sqlx::query_scalar(
        "INSERT INTO proxy_profiles (name, kind, host, port, username) VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&p.name)
    .bind(p.kind.as_str())
    .bind(&p.host)
    .bind(i64::from(p.port))
    .bind(p.username.as_deref())
    .fetch_one(pool)
    .await
    .map_err(|e| name_taken(e, &p.name))
}

/// Returns whether the profile exists.
pub(crate) async fn update_profile(pool: &Pool, id: i64, p: &ProxyInput) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE proxy_profiles SET name = ?, kind = ?, host = ?, port = ?, username = ? WHERE id = ?",
    )
    .bind(&p.name)
    .bind(p.kind.as_str())
    .bind(&p.host)
    .bind(i64::from(p.port))
    .bind(p.username.as_deref())
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| name_taken(e, &p.name))?;
    Ok(result.rows_affected() > 0)
}

/// Record whether the profile has a password in the secret store.
pub(crate) async fn set_has_password(pool: &Pool, id: i64, has: bool) -> Result<()> {
    sqlx::query("UPDATE proxy_profiles SET keyring_ref = ? WHERE id = ?")
        .bind(has.then(|| secret_key(id)))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a profile; its rules go with it (foreign key). Returns whether it existed.
pub(crate) async fn delete_profile(pool: &Pool, id: i64) -> Result<bool> {
    let result = sqlx::query("DELETE FROM proxy_profiles WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn load_settings(pool: &Pool) -> Result<ProxySettings> {
    let rules: Vec<(String, Option<i64>)> =
        sqlx::query_as("SELECT host_pattern, profile_id FROM proxy_rules ORDER BY position, id")
            .fetch_all(pool)
            .await?;
    Ok(ProxySettings {
        default: settings::get::<Option<i64>>(pool, DEFAULT_KEY)
            .await?
            .flatten(),
        rules: rules
            .into_iter()
            .map(|(pattern, profile)| ProxyRule { pattern, profile })
            .collect(),
    })
}

pub(crate) async fn save_settings(pool: &Pool, s: &ProxySettings) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM proxy_rules")
        .execute(&mut *tx)
        .await?;
    for (position, rule) in s.rules.iter().enumerate() {
        sqlx::query(
            "INSERT INTO proxy_rules (host_pattern, profile_id, position) VALUES (?, ?, ?)",
        )
        .bind(&rule.pattern)
        .bind(rule.profile)
        .bind(position as i64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    settings::set(pool, DEFAULT_KEY, &s.default).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ProxyInput {
        ProxyInput {
            name: "  Home  ".into(),
            kind: ProxyKind::Socks5,
            host: " 10.0.0.2 ".into(),
            port: 1080,
            username: Some("  ".into()),
            password: None,
        }
    }

    #[test]
    fn input_is_trimmed_and_checked() {
        let ok = input().validated().unwrap();
        assert_eq!((ok.name.as_str(), ok.host.as_str()), ("Home", "10.0.0.2"));
        assert_eq!(ok.username, None);
        assert!(
            ProxyInput {
                host: "::1".into(),
                ..input()
            }
            .validated()
            .is_ok()
        );
        for bad in [
            ProxyInput {
                name: " ".into(),
                ..input()
            },
            ProxyInput {
                host: "http://proxy".into(),
                ..input()
            },
            ProxyInput {
                host: "proxy:8080".into(),
                ..input()
            },
            ProxyInput {
                host: "a b".into(),
                ..input()
            },
            ProxyInput { port: 0, ..input() },
            ProxyInput {
                username: Some("a:b".into()),
                ..input()
            },
        ] {
            assert!(bad.clone().validated().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn hosts_match_patterns() {
        assert!(host_matches("example.com", "example.com"));
        assert!(host_matches("example.com", "cdn.Example.COM"));
        assert!(!host_matches("example.com", "badexample.com"));
        assert!(!host_matches("example.com", "example.com.evil.net"));
        assert!(host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(host_matches("cdn*.example.net", "cdn7.example.net"));
        assert!(host_matches("10.0.0.5", "10.0.0.5"));
    }

    #[test]
    fn first_matching_rule_wins_then_the_default() {
        let s = ProxySettings {
            default: Some(1),
            rules: vec![
                ProxyRule {
                    pattern: "local.test".into(),
                    profile: None,
                },
                ProxyRule {
                    pattern: "*.video.net".into(),
                    profile: Some(2),
                },
                ProxyRule {
                    pattern: "video.net".into(),
                    profile: Some(3),
                },
            ],
        };
        assert_eq!(s.profile_for("files.local.test"), None);
        assert_eq!(s.profile_for("a.video.net"), Some(2));
        assert_eq!(s.profile_for("video.net"), Some(3));
        assert_eq!(s.profile_for("other.org"), Some(1));
        assert_eq!(ProxySettings::default().profile_for("other.org"), None);
    }

    #[test]
    fn settings_are_cleaned_and_must_name_known_profiles() {
        let s = ProxySettings {
            default: None,
            rules: vec![ProxyRule {
                pattern: " https://Example.com/ ".into(),
                profile: Some(4),
            }],
        };
        assert_eq!(
            s.clone().validated(&[4]).unwrap().rules[0].pattern,
            "example.com"
        );
        assert!(s.clone().validated(&[5]).is_err());
        let blank = ProxySettings {
            default: None,
            rules: vec![ProxyRule {
                pattern: "  ".into(),
                profile: None,
            }],
        };
        assert!(blank.validated(&[]).is_err());
        let gone = ProxySettings {
            default: Some(9),
            rules: vec![],
        };
        assert!(gone.validated(&[1]).is_err());
    }

    #[test]
    fn urls_carry_an_escaped_login() {
        let mut p = ProxyProfile {
            id: 1,
            name: "x".into(),
            kind: ProxyKind::Socks5,
            host: "proxy.lan".into(),
            port: 1080,
            username: Some("me@home".into()),
            has_password: true,
        };
        assert_eq!(
            proxy_url(&p, Some("p:ss/wörd")),
            "socks5h://me%40home:p%3Ass%2Fw%C3%B6rd@proxy.lan:1080"
        );
        p.kind = ProxyKind::Http;
        p.username = None;
        p.host = "::1".into();
        assert_eq!(proxy_url(&p, Some("ignored")), "http://[::1]:1080");
    }

    #[test]
    fn choices_round_trip_through_the_db() {
        for c in [
            ProxyChoice::Auto,
            ProxyChoice::Direct,
            ProxyChoice::Profile { id: 12 },
        ] {
            assert_eq!(ProxyChoice::from_db(c.to_db().as_deref()), c);
        }
        assert_eq!(ProxyChoice::from_db(Some("junk")), ProxyChoice::Auto);
    }
}
