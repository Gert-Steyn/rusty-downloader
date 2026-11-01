//! Which connection a request takes: directly, or through the proxy a
//! download chose or the rules pick for its site (see [`crate::proxy`]),
//! and from which local address when everything is bound to one adapter
//! (a VPN's; see [`crate::adapters`]). One HTTP client per proxy is kept and
//! reused; changing the profiles, rules or binding drops them, so new
//! requests use the new settings while running ones finish on their old
//! connection. While the bound adapter is down nothing connects at all,
//! so nothing slips around the VPN.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use reqwest::Client;
use url::Url;

use crate::db::Pool;
use crate::proxy::{self, ProxyChoice, ProxyInput, ProxyProfile, ProxySettings};
use crate::secrets::{MemorySecrets, Secrets};
use crate::{Error, Result, http};

/// A proxy picked for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRoute {
    pub id: i64,
    pub name: String,
    /// With the login, for reqwest and the tools' `--proxy`. Never log it.
    pub url: String,
}

/// Where connections are sent from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Binding {
    /// Wherever Windows routes them.
    #[default]
    Any,
    /// From this address, so through the adapter that has it.
    Bound(IpAddr),
    /// The chosen adapter (named) is down: no connections at all.
    Down(String),
}

/// How yt-dlp and gallery-dl connect: `--proxy` and `--source-address`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolConnection {
    /// Proxy URL with its login. Never log it.
    pub proxy: Option<String>,
    pub source_address: Option<IpAddr>,
}

impl ToolConnection {
    /// The options for both tools (they spell them the same).
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(proxy) = &self.proxy {
            args.extend(["--proxy".into(), proxy.clone()]);
        }
        if let Some(ip) = self.source_address {
            args.extend(["--source-address".into(), ip.to_string()]);
        }
        args
    }
}

/// Cheap to clone; all clones share the same settings and clients.
#[derive(Clone)]
pub struct Network {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Network {
    // Never the proxy URLs: they carry passwords.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state();
        f.debug_struct("Network")
            .field("profiles", &state.profiles.len())
            .field("rules", &state.settings.rules.len())
            .finish()
    }
}

struct Inner {
    /// `None` for [`Network::direct`], which can't be configured.
    pool: Option<Pool>,
    secrets: Arc<dyn Secrets>,
    user_agent: Option<String>,
    state: RwLock<State>,
    binding: RwLock<Binding>,
    /// By proxy URL (`None`: direct) and whether redirects are followed.
    clients: Mutex<HashMap<(Option<String>, bool), Client>>,
}

#[derive(Default)]
struct State {
    profiles: Vec<ProxyProfile>,
    /// Proxy URL per profile id, login included.
    urls: HashMap<i64, String>,
    settings: ProxySettings,
}

impl Network {
    /// No proxies: every request goes directly. For tests and tools that run
    /// without the app's database.
    pub fn direct(user_agent: Option<&str>) -> Self {
        Self::new(None, MemorySecrets::new(), user_agent)
    }

    /// Profiles and rules from the database, passwords from `secrets`.
    pub async fn load(
        pool: Pool,
        secrets: Arc<dyn Secrets>,
        user_agent: Option<&str>,
    ) -> Result<Self> {
        let network = Self::new(Some(pool), secrets, user_agent);
        network.reload().await?;
        Ok(network)
    }

    fn new(pool: Option<Pool>, secrets: Arc<dyn Secrets>, user_agent: Option<&str>) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                secrets,
                user_agent: user_agent.map(str::to_owned),
                state: RwLock::new(State::default()),
                binding: RwLock::new(Binding::Any),
                clients: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn pool(&self) -> Result<&Pool> {
        self.inner
            .pool
            .as_ref()
            .ok_or_else(|| Error::Unsupported("proxies without a database".into()))
    }

    fn state(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.inner.state.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Read everything again and drop the cached clients.
    async fn reload(&self) -> Result<()> {
        let pool = self.pool()?;
        let profiles = proxy::profiles(pool).await?;
        let settings = proxy::load_settings(pool).await?;
        let mut urls = HashMap::new();
        for p in &profiles {
            let password = if p.has_password {
                // A password that can't be read is reported, not guessed at:
                // the proxy then asks for it and the request fails visibly.
                self.inner
                    .secrets
                    .get(&proxy::secret_key(p.id))
                    .unwrap_or_else(|e| {
                        tracing::warn!(proxy = p.id, error = %e, "could not read proxy password");
                        None
                    })
            } else {
                None
            };
            urls.insert(p.id, proxy::proxy_url(p, password.as_deref()));
        }
        *self.inner.state.write().unwrap_or_else(|e| e.into_inner()) = State {
            profiles,
            urls,
            settings,
        };
        self.inner
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        Ok(())
    }

    /// The proxy a request to `url` takes, `None` for a direct connection.
    /// A chosen profile that no longer exists falls back to the rules.
    pub fn route(&self, url: &Url, choice: ProxyChoice) -> Option<ProxyRoute> {
        let state = self.state();
        let id = match choice {
            ProxyChoice::Direct => return None,
            ProxyChoice::Profile { id } if state.urls.contains_key(&id) => Some(id),
            _ => url
                .host_str()
                .and_then(|host| state.settings.profile_for(host)),
        }?;
        let profile = state.profiles.iter().find(|p| p.id == id)?;
        Some(ProxyRoute {
            id,
            name: profile.name.clone(),
            url: state.urls.get(&id)?.clone(),
        })
    }

    pub fn binding(&self) -> Binding {
        self.inner
            .binding
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Send connections from somewhere else from now on. Returns whether
    /// that changed anything; if so, the cached clients are dropped.
    pub fn set_binding(&self, binding: Binding) -> bool {
        let mut current = self
            .inner
            .binding
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if *current == binding {
            return false;
        }
        tracing::info!(from = ?*current, to = ?binding, "network binding changed");
        *current = binding;
        drop(current);
        self.inner
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        true
    }

    /// The address to send from; an error while the bound adapter is down.
    fn local_address(&self) -> Result<Option<IpAddr>> {
        match self.binding() {
            Binding::Any => Ok(None),
            Binding::Bound(ip) => Ok(Some(ip)),
            Binding::Down(adapter) => Err(Error::NetworkDown(adapter)),
        }
    }

    /// How yt-dlp or gallery-dl should connect for `url`; an error while
    /// the bound adapter is down.
    pub fn tool_connection(&self, url: &Url, choice: ProxyChoice) -> Result<ToolConnection> {
        Ok(ToolConnection {
            source_address: self.local_address()?,
            proxy: self.route(url, choice).map(|r| r.url),
        })
    }

    /// Client for downloads and link checks to `url`: redirects are left
    /// to [`crate::http`], which follows them on the same connection type.
    pub fn client(&self, url: &Url, choice: ProxyChoice) -> Result<Client> {
        self.client_via(self.route(url, choice), false)
    }

    /// Client for API calls and tool downloads to `url`, following redirects.
    pub fn api_client(&self, url: &Url) -> Result<Client> {
        self.client_via(self.route(url, ProxyChoice::Auto), true)
    }

    fn client_via(&self, route: Option<ProxyRoute>, follow: bool) -> Result<Client> {
        let local = self.local_address()?;
        let key = (route.map(|r| r.url), follow);
        let mut clients = self.inner.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }
        let mut builder =
            http::builder(self.inner.user_agent.as_deref(), follow).local_address(local);
        if let Some(url) = &key.0 {
            builder = builder.proxy(reqwest::Proxy::all(url)?);
        }
        let client = builder.build()?;
        clients.insert(key, client.clone());
        Ok(client)
    }

    pub fn profiles(&self) -> Vec<ProxyProfile> {
        self.state().profiles.clone()
    }

    pub fn settings(&self) -> ProxySettings {
        self.state().settings.clone()
    }

    pub async fn add_profile(&self, input: ProxyInput) -> Result<i64> {
        let input = input.validated()?;
        let id = proxy::insert_profile(self.pool()?, &input).await?;
        self.store_password(id, input.password.as_deref()).await?;
        tracing::info!(proxy = id, name = %input.name, kind = input.kind.as_str(), "proxy added");
        self.reload().await?;
        Ok(id)
    }

    pub async fn update_profile(&self, id: i64, input: ProxyInput) -> Result<()> {
        let input = input.validated()?;
        if !proxy::update_profile(self.pool()?, id, &input).await? {
            return Err(Error::NotFound(format!("proxy {id}")));
        }
        self.store_password(id, input.password.as_deref()).await?;
        tracing::info!(proxy = id, name = %input.name, "proxy changed");
        self.reload().await
    }

    /// `None` keeps the stored password, an empty one deletes it.
    async fn store_password(&self, id: i64, password: Option<&str>) -> Result<()> {
        let Some(password) = password else {
            return Ok(());
        };
        let key = proxy::secret_key(id);
        if password.is_empty() {
            self.inner.secrets.delete(&key)?;
        } else {
            self.inner.secrets.set(&key, password)?;
        }
        proxy::set_has_password(self.pool()?, id, !password.is_empty()).await
    }

    /// Delete a profile with its rules and password. If it was the default,
    /// connections go direct again. Downloads that chose it fall back to the rules.
    pub async fn remove_profile(&self, id: i64) -> Result<()> {
        let pool = self.pool()?;
        let mut settings = proxy::load_settings(pool).await?;
        if !proxy::delete_profile(pool, id).await? {
            return Err(Error::NotFound(format!("proxy {id}")));
        }
        if settings.default == Some(id) {
            settings.default = None;
        }
        settings.rules.retain(|r| r.profile != Some(id));
        proxy::save_settings(pool, &settings).await?;
        if let Err(e) = self.inner.secrets.delete(&proxy::secret_key(id)) {
            tracing::warn!(proxy = id, error = %e, "could not delete proxy password");
        }
        tracing::info!(proxy = id, "proxy removed");
        self.reload().await
    }

    pub async fn set_settings(&self, settings: ProxySettings) -> Result<()> {
        let ids: Vec<i64> = self.state().profiles.iter().map(|p| p.id).collect();
        let settings = settings.validated(&ids)?;
        proxy::save_settings(self.pool()?, &settings).await?;
        tracing::info!(
            default = ?settings.default,
            rules = settings.rules.len(),
            "proxy rules changed"
        );
        self.reload().await
    }

    /// Fetch `url` through `choice` (redirects followed) and say how long
    /// the answer took. Any HTTP answer counts: the proxy worked.
    pub async fn test(&self, url: &Url, choice: ProxyChoice) -> Result<Duration> {
        let client = self.client_via(self.route(url, choice), true)?;
        let started = Instant::now();
        let response =
            tokio::time::timeout(Duration::from_secs(20), client.get(url.clone()).send())
                .await
                .map_err(|_| {
                    Error::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no answer within 20 seconds",
                    ))
                })?
                .map_err(|e| Error::InvalidInput(with_causes(&e)))?;
        if response.status() == reqwest::StatusCode::PROXY_AUTHENTICATION_REQUIRED {
            return Err(Error::InvalidInput(
                "The proxy wants a login (HTTP 407): check the user name and password.".into(),
            ));
        }
        tracing::info!(
            status = response.status().as_u16(),
            ?choice,
            "proxy test answered"
        );
        Ok(started.elapsed())
    }
}

/// An error with its causes, which reqwest keeps out of its own message
/// ("error sending request" says nothing about a refused proxy login).
fn with_causes(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if !text.contains(&message) {
            text.push_str(": ");
            text.push_str(&message);
        }
        source = cause.source();
    }
    text
}
