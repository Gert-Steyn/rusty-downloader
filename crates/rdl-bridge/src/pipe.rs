//! The named pipe: the app serves it, the native host connects to it.
//!
//! The name includes the Windows user name, so two users signed in at once
//! each get their own app. Remote clients are refused, and the first
//! instance must be ours: if something else already holds the name, binding
//! fails instead of sharing it. Each connection may send any number of
//! requests, one at a time; it is closed after a minute without one.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::frame::{read_frame, write_frame};
use crate::protocol::{Message, Reply, Request, decode_request};

/// Connections served at the same time; more are closed straight away.
const MAX_CONNECTIONS: usize = 8;

/// A connection that sends nothing for this long is closed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a client waits while every pipe instance is busy.
const BUSY_WAIT: Duration = Duration::from_secs(5);

/// `ERROR_PIPE_BUSY`: the server exists but has no free instance right now.
const ERROR_PIPE_BUSY: i32 = 231;

/// `\\.\pipe\rusty-downloader-<user>`, from `%USERNAME%` (reduced to
/// characters safe in a pipe name).
pub fn pipe_name() -> String {
    let user: String = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let user = if user.is_empty() { "user".into() } else { user };
    format!(r"\\.\pipe\rusty-downloader-{user}")
}

/// Answers requests. The app implements this; tests use a fake.
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, request: Request) -> impl Future<Output = Reply> + Send;
}

/// A bound pipe, ready to [`run`](Server::run).
pub struct Server {
    name: String,
    next: NamedPipeServer,
}

impl Server {
    /// Claim the pipe name. Fails if another process (or another copy of
    /// the app) already has it.
    pub fn bind(name: &str) -> io::Result<Self> {
        let next = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(name)?;
        Ok(Self {
            name: name.to_owned(),
            next,
        })
    }

    /// Serve until `shutdown` is cancelled.
    pub async fn run<H: Handler>(self, handler: Arc<H>, shutdown: CancellationToken) {
        let Self { name, mut next } = self;
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            let connected = tokio::select! {
                _ = shutdown.cancelled() => return,
                connected = next.connect() => connected,
            };
            // A fresh instance takes over listening before this one is served.
            let replacement = match ServerOptions::new()
                .reject_remote_clients(true)
                .create(&name)
            {
                Ok(server) => server,
                Err(e) => {
                    tracing::error!(error = %e, "could not reopen the browser pipe; stopped");
                    return;
                }
            };
            let pipe = std::mem::replace(&mut next, replacement);
            if let Err(e) = connected {
                tracing::debug!(error = %e, "pipe client went away while connecting");
                continue;
            }
            let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                tracing::warn!("too many browser pipe connections; closing one");
                continue;
            };
            let handler = Arc::clone(&handler);
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                let _slot = slot;
                if let Err(e) = session(pipe, &*handler, &shutdown).await {
                    tracing::debug!(error = %e, "pipe connection ended with an error");
                }
            });
        }
    }
}

/// Requests and replies on one connection, until it closes or idles out.
async fn session<H: Handler>(
    mut pipe: NamedPipeServer,
    handler: &H,
    shutdown: &CancellationToken,
) -> io::Result<()> {
    loop {
        let frame = tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            frame = tokio::time::timeout(IDLE_TIMEOUT, read_frame(&mut pipe)) => frame,
        };
        let bytes = match frame {
            Err(_idle) => return Ok(()),
            Ok(Ok(Some(bytes))) => bytes,
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(e)) if e.kind() == io::ErrorKind::InvalidData => {
                // Can't find the next message after a bad length: say why, then hang up.
                let reply = Message::new(None, Reply::error(e.to_string()));
                write_frame(&mut pipe, &serde_json::to_vec(&reply)?).await?;
                return Err(e);
            }
            Ok(Err(e)) => return Err(e),
        };
        let reply = match decode_request(&bytes) {
            Ok(message) => {
                tracing::debug!(id = message.id, request = ?kind(&message.body), "browser request");
                Message::new(message.id, handler.handle(message.body).await)
            }
            Err(reply) => {
                tracing::info!(?reply, "refused a browser pipe message");
                reply
            }
        };
        write_frame(&mut pipe, &serde_json::to_vec(&reply)?).await?;
    }
}

/// The request's type, for logs (an `add` holds cookies, which stay out).
fn kind(request: &Request) -> &'static str {
    match request {
        Request::Ping => "ping",
        Request::Status => "status",
        Request::Add(_) => "add",
    }
}

/// Connect to the app's pipe, waiting briefly while it's busy. A
/// `NotFound` error means the app isn't running.
pub async fn connect(name: &str) -> io::Result<NamedPipeClient> {
    let deadline = tokio::time::Instant::now() + BUSY_WAIT;
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(client),
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY)
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Send one raw message and read the raw reply.
pub async fn exchange(client: &mut NamedPipeClient, message: &[u8]) -> io::Result<Vec<u8>> {
    write_frame(client, message).await?;
    read_frame(client).await?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the app closed the connection without replying",
        )
    })
}

/// Send one request and read the reply.
pub async fn call(
    client: &mut NamedPipeClient,
    request: &Message<Request>,
) -> io::Result<Message<Reply>> {
    let reply = exchange(client, &serde_json::to_vec(request)?).await?;
    serde_json::from_slice(&reply).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_is_per_user_and_safe() {
        let name = pipe_name();
        let user = name.strip_prefix(r"\\.\pipe\rusty-downloader-").unwrap();
        assert!(!user.is_empty());
        assert!(!user.contains(['\\', '/', ' ']));
    }
}
