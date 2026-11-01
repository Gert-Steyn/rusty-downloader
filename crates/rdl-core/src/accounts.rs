//! Site accounts: a login and/or cookies for a site. The password lives in
//! [`crate::secrets`] under `account/<id>`; the cookies, imported from a
//! `cookies.txt`, are stored sealed (DPAPI, this Windows user only) in
//! `accounts.cookies_sealed`. [`crate::network::Network`] keeps them loaded
//! and hands them out per URL: cookies are merged into requests, the login
//! goes to yt-dlp / gallery-dl and answers a site's HTTP Basic challenge.

use serde::{Deserialize, Serialize};

use crate::db::Pool;
use crate::proxy::host_matches;
use crate::request::Cookie;
use crate::secrets::Secrets;
use crate::{Error, Result};

/// An account as the UI sees it: never the password or cookie values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: i64,
    /// `example.com` (with its subdomains) or a `*` pattern, as in proxy rules.
    pub site: String,
    pub username: Option<String>,
    pub has_password: bool,
    /// Cookies stored for it (expired ones aren't counted).
    pub cookies: u32,
    pub enabled: bool,
}

/// A new or changed account.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AccountInput {
    pub site: String,
    pub username: Option<String>,
    /// `None` keeps the stored password; an empty one removes it.
    pub password: Option<String>,
    pub enabled: bool,
}

impl AccountInput {
    pub fn validated(mut self) -> Result<Self> {
        self.site = self
            .site
            .trim()
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        self.username = self
            .username
            .map(|u| u.trim().to_owned())
            .filter(|u| !u.is_empty());
        if self.site.is_empty() || self.site.contains(['/', ' ', '?', '#', '@']) {
            return Err(Error::InvalidInput(
                "Enter the site as example.com or *.example.com.".into(),
            ));
        }
        if self.username.is_none() && self.password.as_deref().is_some_and(|p| !p.is_empty()) {
            return Err(Error::InvalidInput("A password needs a user name.".into()));
        }
        Ok(self)
    }
}

/// A user name and password for a site.
#[derive(Clone, PartialEq, Eq)]
pub struct Login {
    /// The account's site pattern: the login is only ever given there.
    pub site: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Login {
    // Never the password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("site", &self.site)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl Login {
    /// Whether `host` belongs to the login's site.
    pub fn is_for(&self, host: &str) -> bool {
        host_matches(&self.site, host)
    }
}

/// An account with what it hands out, loaded for use.
#[derive(Debug, Clone)]
pub(crate) struct Loaded {
    pub account: Account,
    pub login: Option<Login>,
    pub cookies: Vec<Cookie>,
}

/// The enabled account for `host`: the most specific matching site wins
/// (`video.example.com` before `example.com`), then the oldest.
pub(crate) fn for_host<'a>(accounts: &'a [Loaded], host: &str) -> Option<&'a Loaded> {
    accounts
        .iter()
        .filter(|a| a.account.enabled && host_matches(&a.account.site, host))
        .max_by_key(|a| (a.account.site.len(), std::cmp::Reverse(a.account.id)))
}

/// Which of `cookies` belong to `site`: their domain is the site's, one of
/// its subdomains, or a parent domain the site would receive them from.
pub fn cookies_for_site(site: &str, cookies: Vec<Cookie>) -> Vec<Cookie> {
    let base = site.trim_start_matches("*.").trim_start_matches('*');
    cookies
        .into_iter()
        .filter(|c| {
            let domain = c.bare_domain();
            host_matches(site, &domain)
                || host_matches(&domain, base)
                || (!base.contains('*') && host_matches(base, &domain))
        })
        .collect()
}

/// Where an account's password is kept in [`crate::secrets`].
pub fn secret_key(id: i64) -> String {
    format!("account/{id}")
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    site: String,
    username: String,
    keyring_ref: Option<String>,
    cookies_sealed: Option<Vec<u8>>,
    enabled: i64,
}

/// Every account with its secrets; one whose password or cookies can't be
/// read is loaded without them (logged).
pub(crate) async fn load(pool: &Pool, secrets: &dyn Secrets) -> Result<Vec<Loaded>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, site, username, keyring_ref, cookies_sealed, enabled FROM accounts ORDER BY site, id",
    )
    .fetch_all(pool)
    .await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(rows
        .into_iter()
        .map(|r| {
            let password = r.keyring_ref.as_ref().and_then(|_| {
                secrets.get(&secret_key(r.id)).unwrap_or_else(|e| {
                    tracing::warn!(account = r.id, error = %e, "could not read account password");
                    None
                })
            });
            let cookies: Vec<Cookie> = r
                .cookies_sealed
                .as_deref()
                .and_then(|sealed| match secrets.open(sealed) {
                    Ok(json) => serde_json::from_slice(&json).ok(),
                    Err(e) => {
                        tracing::warn!(account = r.id, error = %e, "could not read account cookies");
                        None
                    }
                })
                .unwrap_or_default();
            let username = Some(r.username).filter(|u| !u.is_empty());
            let login = match (&username, password) {
                (Some(user), Some(password)) => Some(Login {
                    site: r.site.clone(),
                    username: user.clone(),
                    password,
                }),
                _ => None,
            };
            Loaded {
                account: Account {
                    id: r.id,
                    site: r.site,
                    username,
                    has_password: r.keyring_ref.is_some(),
                    cookies: cookies
                        .iter()
                        .filter(|c| c.expires.is_none_or(|e| e > now))
                        .count() as u32,
                    enabled: r.enabled != 0,
                },
                login,
                cookies,
            }
        })
        .collect())
}

/// A clash with another account for the same site and user, for the user.
fn taken(e: sqlx::Error, input: &AccountInput) -> Error {
    if e.as_database_error()
        .is_some_and(|d| d.is_unique_violation())
    {
        Error::InvalidInput(format!(
            "There is already an account for {}{}.",
            input.site,
            input
                .username
                .as_ref()
                .map(|u| format!(" as {u}"))
                .unwrap_or_default()
        ))
    } else {
        e.into()
    }
}

pub(crate) async fn insert(pool: &Pool, a: &AccountInput) -> Result<i64> {
    sqlx::query_scalar(
        "INSERT INTO accounts (site, username, enabled) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(&a.site)
    .bind(a.username.as_deref().unwrap_or(""))
    .bind(i64::from(a.enabled))
    .fetch_one(pool)
    .await
    .map_err(|e| taken(e, a))
}

/// Returns whether the account exists.
pub(crate) async fn update(pool: &Pool, id: i64, a: &AccountInput) -> Result<bool> {
    let result =
        sqlx::query("UPDATE accounts SET site = ?, username = ?, enabled = ? WHERE id = ?")
            .bind(&a.site)
            .bind(a.username.as_deref().unwrap_or(""))
            .bind(i64::from(a.enabled))
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| taken(e, a))?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn set_has_password(pool: &Pool, id: i64, has: bool) -> Result<()> {
    sqlx::query("UPDATE accounts SET keyring_ref = ? WHERE id = ?")
        .bind(has.then(|| secret_key(id)))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Store the cookies sealed (`None` clears them). Returns whether the account exists.
pub(crate) async fn set_cookies(pool: &Pool, id: i64, sealed: Option<Vec<u8>>) -> Result<bool> {
    let result = sqlx::query("UPDATE accounts SET cookies_sealed = ? WHERE id = ?")
        .bind(sealed)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn delete(pool: &Pool, id: i64) -> Result<bool> {
    let result = sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: "v".into(),
            domain: domain.into(),
            host_only: false,
            path: "/".into(),
            secure: false,
            http_only: false,
            expires: None,
        }
    }

    fn loaded(id: i64, site: &str, enabled: bool) -> Loaded {
        Loaded {
            account: Account {
                id,
                site: site.into(),
                username: None,
                has_password: false,
                cookies: 0,
                enabled,
            },
            login: None,
            cookies: Vec::new(),
        }
    }

    #[test]
    fn input_is_cleaned_and_checked() {
        let input = AccountInput {
            site: " https://Example.com/ ".into(),
            username: Some("  ".into()),
            password: None,
            enabled: true,
        }
        .validated()
        .unwrap();
        assert_eq!((input.site.as_str(), input.username), ("example.com", None));
        let bad = |site: &str, user: Option<&str>, pass: Option<&str>| {
            AccountInput {
                site: site.into(),
                username: user.map(Into::into),
                password: pass.map(Into::into),
                enabled: true,
            }
            .validated()
            .is_err()
        };
        assert!(bad("", None, None));
        assert!(bad("example.com/path", None, None));
        assert!(bad("example.com", None, Some("secret")));
        assert!(!bad("*.example.com", Some("me"), Some("secret")));
    }

    #[test]
    fn the_most_specific_enabled_account_wins() {
        let accounts = vec![
            loaded(1, "example.com", true),
            loaded(2, "video.example.com", true),
            loaded(3, "other.org", false),
        ];
        assert_eq!(
            for_host(&accounts, "video.example.com").unwrap().account.id,
            2
        );
        assert_eq!(
            for_host(&accounts, "cdn.example.com").unwrap().account.id,
            1
        );
        assert!(for_host(&accounts, "other.org").is_none(), "disabled");
        assert!(for_host(&accounts, "elsewhere.net").is_none());
    }

    #[test]
    fn imported_cookies_are_kept_for_their_site_only() {
        let all = vec![
            cookie("a", ".example.com"),
            cookie("b", "www.example.com"),
            cookie("c", "tracker.net"),
            cookie("d", "badexample.com"),
        ];
        let names = |site: &str| -> Vec<String> {
            cookies_for_site(site, all.clone())
                .into_iter()
                .map(|c| c.name)
                .collect()
        };
        assert_eq!(names("example.com"), ["a", "b"]);
        // A parent domain's cookies reach the subdomain too.
        assert_eq!(names("www.example.com"), ["a", "b"]);
        assert_eq!(names("*.example.com"), ["a", "b"]);
    }

    #[test]
    fn logins_are_only_for_their_site_and_never_printed() {
        let login = Login {
            site: "example.com".into(),
            username: "me".into(),
            password: "hunter2".into(),
        };
        assert!(login.is_for("files.example.com"));
        assert!(!login.is_for("example.org"));
        assert!(!format!("{login:?}").contains("hunter2"));
    }
}
