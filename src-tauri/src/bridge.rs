//! The app's end of the browser bridge: serves the named pipe the native
//! host relays extension messages over (see `rdl-bridge`).
//!
//! `add` puts links in the LinkGrabber (with a notification when the app is
//! in the background), or for a download the browser was about to save,
//! straight into the queue. Either way the referer, user agent and cookies
//! travel with the links. Everything in a message is checked again here:
//! links must be http(s), and the request context is cleaned by `rdl-core`.

use std::sync::Arc;

use rdl_bridge::pipe::{self, Handler, Server};
use rdl_bridge::{AddRequest, Reply, Request, Target};
use rdl_core::linkgrabber::{LinkSource, default_package_name};
use rdl_core::manager::{DEFAULT_QUEUE, NewDownload, NewPackage, Package, Status};
use rdl_core::request::{Cookie, RequestContext};
use tauri::{AppHandle, Manager};

use crate::commands::parse_links;
use crate::state::AppState;

/// Serve the pipe until the app exits. If the name is taken (by another
/// copy of the app, or anything else), the browser can't reach this one;
/// that's logged, and the app works on without it.
pub fn start(app: &AppHandle) {
    let shutdown = app.state::<AppState>().shutdown.clone();
    let handler = Arc::new(AppHandler { app: app.clone() });
    // Binding registers the pipe with the runtime, so it happens inside it.
    tauri::async_runtime::spawn(async move {
        let name = pipe::pipe_name();
        let server = match Server::bind(&name) {
            Ok(server) => server,
            Err(e) => {
                tracing::error!(pipe = %name, error = %e, "could not open the browser pipe");
                return;
            }
        };
        tracing::info!(pipe = %name, "listening for the browser");
        server.run(handler, shutdown).await;
    });
}

struct AppHandler {
    app: AppHandle,
}

impl Handler for AppHandler {
    async fn handle(&self, request: Request) -> Reply {
        let result = match request {
            Request::Ping => Ok(Reply::Pong {
                app: "rusty-downloader".into(),
                version: rdl_core::VERSION.into(),
                protocol: rdl_bridge::PROTOCOL_VERSION,
            }),
            Request::Status => self.status().await,
            Request::Add(add) => self.add(add).await,
        };
        result.unwrap_or_else(|message| {
            tracing::warn!(error = %message, "browser request failed");
            Reply::error(message)
        })
    }
}

impl AppHandler {
    async fn status(&self) -> Result<Reply, String> {
        let state = self.app.state::<AppState>();
        let packages = state.downloads.list().await.map_err(|e| e.to_string())?;
        let (active, queued) = count_running(&packages);
        let grabber = state.grabber.list().await.map_err(|e| e.to_string())?.len();
        Ok(Reply::Status {
            active,
            queued,
            grabber: grabber as u32,
        })
    }

    async fn add(&self, add: AddRequest) -> Result<Reply, String> {
        let state = self.app.state::<AppState>();
        let (urls, rejected) = parse_links(&add.links);
        if urls.is_empty() {
            return Err("None of the links are http or https links.".into());
        }
        let request = request_context(&add);
        let count = urls.len();
        tracing::info!(
            links = count,
            target = ?add.target,
            cookies = request.cookies.len(),
            "links from the browser"
        );
        match add.target {
            Target::Grabber => {
                let outcome = state
                    .grabber
                    .add_with_request(urls, LinkSource::Browser, request)
                    .await
                    .map_err(|e| e.to_string())?;
                crate::notify::links_added(&self.app, outcome.added.len());
                Ok(Reply::Added {
                    target: Target::Grabber,
                    added: outcome.added.len() as u32,
                    duplicates: outcome.duplicates as u32,
                    rejected,
                })
            }
            Target::Queue => {
                let dest_dir = state.download_dir(&self.app)?;
                let name = add
                    .package
                    .as_deref()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| default_package_name(&urls));
                // The browser's file name only makes sense for a single link.
                let filename = add.filename.clone().filter(|_| count == 1);
                let downloads = urls
                    .into_iter()
                    .map(|url| NewDownload {
                        filename: filename.clone(),
                        request: request.clone(),
                        ..NewDownload::new(url)
                    })
                    .collect();
                state
                    .downloads
                    .add_package(NewPackage {
                        name,
                        dest_dir,
                        queue_id: DEFAULT_QUEUE,
                        downloads,
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(Reply::Added {
                    target: Target::Queue,
                    added: count as u32,
                    duplicates: 0,
                    rejected,
                })
            }
        }
    }
}

/// What the browser sent along with the links, cleaned.
fn request_context(add: &AddRequest) -> RequestContext {
    RequestContext {
        referer: add.referer.clone(),
        user_agent: add.user_agent.clone(),
        cookies: add
            .cookies
            .iter()
            .map(|c| Cookie {
                name: c.name.clone(),
                value: c.value.clone(),
                domain: c.domain.clone(),
                host_only: c.host_only,
                path: c.path.clone(),
                secure: c.secure,
                http_only: c.http_only,
                expires: c.expires(),
            })
            .collect(),
    }
    .cleaned()
}

/// Downloads running and waiting to run.
fn count_running(packages: &[Package]) -> (u32, u32) {
    let downloads = packages.iter().flat_map(|p| &p.downloads);
    let (mut active, mut queued) = (0, 0);
    for d in downloads {
        match d.status {
            Status::Active => active += 1,
            Status::Queued => queued += 1,
            _ => {}
        }
    }
    (active, queued)
}

#[cfg(test)]
mod tests {
    use rdl_bridge::BrowserCookie;

    use super::*;

    #[test]
    fn browser_cookies_become_request_cookies() {
        let cookie = |name: &str, session: bool| BrowserCookie {
            name: name.into(),
            value: "v".into(),
            domain: ".Example.com".into(),
            host_only: false,
            path: "/".into(),
            secure: true,
            http_only: true,
            session,
            expiration_date: Some(4_000_000_000.9),
        };
        let add = AddRequest {
            links: vec!["https://example.com/f".into()],
            target: Target::Grabber,
            referer: Some("https://example.com/page".into()),
            user_agent: Some("Browser/1".into()),
            cookies: vec![
                cookie("kept", false),
                cookie("session", true),
                cookie("bad;name", false),
            ],
            filename: None,
            package: None,
        };
        let request = request_context(&add);
        assert_eq!(request.referer.as_deref(), Some("https://example.com/page"));
        assert_eq!(request.user_agent.as_deref(), Some("Browser/1"));
        let names: Vec<(&str, Option<i64>)> = request
            .cookies
            .iter()
            .map(|c| (c.name.as_str(), c.expires))
            .collect();
        assert_eq!(
            names,
            [("kept", Some(4_000_000_000)), ("session", None)],
            "malformed cookies are dropped"
        );
        assert_eq!(request.cookies[0].domain, ".example.com");
    }
}
