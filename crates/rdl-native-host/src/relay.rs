//! Passing the browser's messages to the app and its replies back.
//!
//! Messages go through unchanged; the app checks them. The host only reads
//! a message's `id` and `type`: the id so it can answer in the app's place
//! when the app can't be reached, the type because only an `add` may start
//! the app (the extension's popup pings to show whether it's running, and
//! that mustn't launch it). One pipe connection is kept for the browser
//! session and reopened when the app has closed it.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use rdl_bridge::frame::{read_frame, write_frame};
use rdl_bridge::pipe;
use rdl_bridge::{ErrorCode, Message, Reply};
use tokio::net::windows::named_pipe::NamedPipeClient;

use crate::{launch, trust};

/// Native messaging won't pass a message over 1 MB from the host to the browser.
const MAX_TO_BROWSER: usize = 1024 * 1024;

/// How long the app may take to answer one request.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// The app's file name, next to the host.
const APP_EXE: &str = "rusty-downloader.exe";

#[derive(Debug, Clone)]
pub struct Config {
    pub pipe_name: String,
    pub app_exe: PathBuf,
    /// How long a just-started app has to open its pipe.
    pub launch_wait: Duration,
}

impl Config {
    /// The app next to this host, on the current user's pipe. Debug builds
    /// take `RDL_HOST_PIPE`, `RDL_HOST_APP` and `RDL_HOST_LAUNCH_WAIT_MS`
    /// instead, for tests; release builds ignore them.
    pub fn from_env() -> io::Result<Self> {
        #[allow(unused_mut)]
        let mut config = Self {
            pipe_name: pipe::pipe_name(),
            app_exe: std::env::current_exe()?.with_file_name(APP_EXE),
            launch_wait: Duration::from_secs(20),
        };
        #[cfg(debug_assertions)]
        {
            if let Ok(name) = std::env::var("RDL_HOST_PIPE") {
                config.pipe_name = name;
            }
            if let Ok(app) = std::env::var("RDL_HOST_APP") {
                config.app_exe = app.into();
            }
            if let Some(ms) = std::env::var("RDL_HOST_LAUNCH_WAIT_MS")
                .ok()
                .and_then(|ms| ms.parse().ok())
            {
                config.launch_wait = Duration::from_millis(ms);
            }
        }
        Ok(config)
    }
}

pub struct Relay {
    config: Config,
    client: Option<NamedPipeClient>,
}

impl Relay {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            client: None,
        }
    }

    /// The reply to one message from the browser: the app's, or an error
    /// in its place.
    pub async fn handle(&mut self, message: &[u8]) -> Vec<u8> {
        let head: Option<serde_json::Value> = serde_json::from_slice(message).ok();
        let field = |name: &str| head.as_ref().and_then(|h| h.get(name));
        let id = field("id").and_then(serde_json::Value::as_u64);
        let may_start = field("type").and_then(serde_json::Value::as_str) == Some("add");

        let reply = match self.forward(message, may_start).await {
            Ok(reply) if reply.len() <= MAX_TO_BROWSER => return reply,
            Ok(reply) => Reply::error_with(
                ErrorCode::Unavailable,
                format!(
                    "the app's reply ({} bytes) is too big for the browser",
                    reply.len()
                ),
            ),
            Err(reply) => reply,
        };
        if let Reply::Error { message, .. } = &reply {
            eprintln!("rdl-native-host: {message}");
        }
        serde_json::to_vec(&Message::new(id, reply)).expect("replies serialise")
    }

    async fn forward(&mut self, message: &[u8], may_start: bool) -> Result<Vec<u8>, Reply> {
        // The kept connection fails on write if the app closed it (idle, or
        // restarted); then it's reopened. Once a message has gone out it is
        // never sent twice, so a lost reply can't add links twice.
        if let Some(client) = &mut self.client
            && write_frame(client, message).await.is_ok()
        {
            return self.read_reply().await;
        }
        self.client = None;
        let mut client = self.connect(may_start).await?;
        write_frame(&mut client, message)
            .await
            .map_err(|e| unavailable(format!("could not send to the app: {e}")))?;
        self.client = Some(client);
        self.read_reply().await
    }

    async fn read_reply(&mut self) -> Result<Vec<u8>, Reply> {
        let client = self.client.as_mut().expect("connected before reading");
        let reply = match tokio::time::timeout(REPLY_TIMEOUT, read_frame(client)).await {
            Ok(Ok(Some(reply))) => return Ok(reply),
            Ok(Ok(None)) => "the app closed the connection without answering".to_owned(),
            Ok(Err(e)) => format!("could not read the app's answer: {e}"),
            Err(_) => "the app took too long to answer".to_owned(),
        };
        self.client = None;
        Err(unavailable(reply))
    }

    /// Open the pipe and check it's the app's. Starts the app when allowed.
    async fn connect(&self, may_start: bool) -> Result<NamedPipeClient, Reply> {
        let name = &self.config.pipe_name;
        let client = match pipe::connect(name).await {
            Ok(client) => client,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !may_start {
                    return Err(Reply::error_with(
                        ErrorCode::NotRunning,
                        "Rusty Downloader isn't running.",
                    ));
                }
                launch::start_app(&self.config.app_exe).map_err(|e| {
                    Reply::error_with(
                        ErrorCode::NotRunning,
                        format!(
                            "Rusty Downloader isn't running and couldn't be started ({}): {e}",
                            self.config.app_exe.display()
                        ),
                    )
                })?;
                self.wait_for_pipe().await?
            }
            Err(e) => return Err(unavailable(format!("could not reach the app: {e}"))),
        };
        trust::check_server(&client, &self.config.app_exe).map_err(|why| {
            Reply::error_with(
                ErrorCode::Untrusted,
                format!("Not sending anything: {why}."),
            )
        })?;
        Ok(client)
    }

    async fn wait_for_pipe(&self) -> Result<NamedPipeClient, Reply> {
        let deadline = tokio::time::Instant::now() + self.config.launch_wait;
        loop {
            match pipe::connect(&self.config.pipe_name).await {
                Ok(client) => return Ok(client),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(unavailable(format!("could not reach the app: {e}"))),
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Reply::error_with(
                    ErrorCode::NotRunning,
                    "Rusty Downloader was started but isn't answering yet. Try again in a moment.",
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

fn unavailable(message: String) -> Reply {
    Reply::error_with(ErrorCode::Unavailable, message)
}
