//! The real host binary, driven over stdin/stdout as a browser would, with
//! this test process serving the pipe in the app's place. Debug builds
//! only: the host reads its pipe, app path and launch wait from the
//! environment there (`RDL_HOST_*`).
#![cfg(all(windows, debug_assertions))]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rdl_bridge::frame::{read_frame, write_frame};
use rdl_bridge::pipe::{Handler, Server};
use rdl_bridge::{ErrorCode, Message, Reply, Request};
use serde_json::{Value, json};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio_util::sync::CancellationToken;

/// Plays the app: answers pings and adds, counting requests.
#[derive(Default)]
struct FakeApp {
    requests: AtomicUsize,
}

impl Handler for FakeApp {
    async fn handle(&self, request: Request) -> Reply {
        self.requests.fetch_add(1, Ordering::SeqCst);
        match request {
            Request::Ping => Reply::Pong {
                app: "fake".into(),
                version: "0".into(),
                protocol: 1,
            },
            Request::Status => Reply::Status {
                active: 0,
                queued: 0,
                grabber: 0,
            },
            Request::Add(add) => Reply::Added {
                target: add.target,
                added: add.links.len() as u32,
                duplicates: 0,
                rejected: Vec::new(),
            },
        }
    }
}

fn unique_pipe() -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    format!(
        r"\\.\pipe\rdl-host-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

/// Serve `name` until the token is cancelled.
fn serve(name: &str, app: Arc<FakeApp>) -> CancellationToken {
    let stop = CancellationToken::new();
    let server = Server::bind(name).unwrap();
    tokio::spawn(server.run(app, stop.clone()));
    stop
}

/// The host, as the browser runs it.
struct Host {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl Host {
    /// `app` is the program the host trusts and would start.
    fn start(pipe: &str, app: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rdl-native-host"))
            .arg("chrome-extension://test/")
            .env("RDL_HOST_PIPE", pipe)
            .env("RDL_HOST_APP", app)
            .env("RDL_HOST_LAUNCH_WAIT_MS", "600")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        Self {
            child,
            stdin,
            stdout,
        }
    }

    async fn send(&mut self, message: Value) -> Value {
        write_frame(&mut self.stdin, message.to_string().as_bytes())
            .await
            .unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut self.stdout))
            .await
            .expect("host answered in time")
            .unwrap()
            .expect("a reply, not the end");
        serde_json::from_slice(&reply).unwrap()
    }

    /// Close stdin, as the browser does, and wait for the host to exit and
    /// its stdout to close. The browser waits for that end of the pipe, so
    /// nothing the host started may still hold it.
    async fn finish(self) -> std::process::ExitStatus {
        let Self {
            mut child,
            stdin,
            mut stdout,
        } = self;
        drop(stdin);
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("host exited")
            .unwrap();
        let end = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut stdout))
            .await
            .expect("the host's stdout closed: something it started still holds it");
        assert!(matches!(end, Ok(None)), "{end:?}");
        status
    }
}

fn ping(id: u64) -> Value {
    json!({"v": 1, "id": id, "type": "ping"})
}

fn add(id: u64) -> Value {
    json!({"v": 1, "id": id, "type": "add", "links": ["https://x.test/a", "https://x.test/b"],
           "cookies": [{"name": "sid", "value": "secret", "domain": "x.test"}]})
}

fn error_code(reply: &Value) -> Option<ErrorCode> {
    let reply: Message<Reply> = serde_json::from_value(reply.clone()).unwrap();
    match reply.body {
        Reply::Error { code, .. } => code,
        other => panic!("expected an error, got {other:?}"),
    }
}

/// A fake app that records that it was started, and with what, then stays
/// around for a few seconds without opening the pipe, as a slow app would.
fn fake_launcher(dir: &Path) -> (PathBuf, PathBuf) {
    let marker = dir.join("started.txt");
    let script = dir.join("fake-app.cmd");
    std::fs::write(
        &script,
        format!(
            "@echo off\r\necho %*>\"{}\"\r\nping -n 6 127.0.0.1 >NUL\r\n",
            marker.display()
        ),
    )
    .unwrap();
    (script, marker)
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rdl-host-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn relays_messages_and_replies() {
    let pipe = unique_pipe();
    let app = Arc::new(FakeApp::default());
    let _stop = serve(&pipe, Arc::clone(&app));
    // The test process serves the pipe, so it's the "app" to trust.
    let mut host = Host::start(&pipe, &std::env::current_exe().unwrap());

    let pong = host.send(ping(1)).await;
    assert_eq!(pong["type"], "pong");
    assert_eq!(pong["id"], 1);
    let added = host.send(add(2)).await;
    assert_eq!(added["type"], "added");
    assert_eq!(added["added"], 2);
    // Passed through untouched, so the app's own errors come back as they are.
    let refused = host
        .send(json!({"v": 1, "id": 3, "type": "format-disk"}))
        .await;
    assert_eq!(refused["type"], "error");
    assert_eq!(refused["id"], 3);
    assert_eq!(app.requests.load(Ordering::SeqCst), 2);

    assert!(host.finish().await.success());
}

#[tokio::test]
async fn sends_nothing_to_a_pipe_the_app_does_not_serve() {
    let pipe = unique_pipe();
    let app = Arc::new(FakeApp::default());
    let _stop = serve(&pipe, Arc::clone(&app));
    // The host trusts a different program than the one serving the pipe.
    let mut host = Host::start(&pipe, Path::new(env!("CARGO_BIN_EXE_rdl-native-host")));

    let reply = host.send(add(7)).await;
    assert_eq!(reply["id"], 7);
    assert_eq!(error_code(&reply), Some(ErrorCode::Untrusted));
    assert!(reply["message"].as_str().unwrap().contains("instead of"));
    assert_eq!(app.requests.load(Ordering::SeqCst), 0, "nothing reached it");
    host.finish().await;
}

#[tokio::test]
async fn a_ping_does_not_start_the_app() {
    let dir = scratch_dir("ping");
    let (launcher, marker) = fake_launcher(&dir);
    let mut host = Host::start(&unique_pipe(), &launcher);

    let reply = host.send(ping(1)).await;
    assert_eq!(error_code(&reply), Some(ErrorCode::NotRunning));
    let status = host.send(json!({"v": 1, "type": "status"})).await;
    assert_eq!(error_code(&status), Some(ErrorCode::NotRunning));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!marker.exists(), "the app was started");
    host.finish().await;
}

#[tokio::test]
async fn an_add_starts_the_app_in_the_tray() {
    let dir = scratch_dir("add");
    let (launcher, marker) = fake_launcher(&dir);
    let mut host = Host::start(&unique_pipe(), &launcher);

    // The fake app never opens the pipe, so the host gives up after its wait.
    let reply = host.send(add(4)).await;
    assert_eq!(reply["id"], 4);
    assert_eq!(error_code(&reply), Some(ErrorCode::NotRunning));
    assert!(reply["message"].as_str().unwrap().contains("was started"));
    let args = std::fs::read_to_string(&marker).expect("the app was started");
    assert_eq!(args.trim(), "--minimized");
    host.finish().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn reconnects_after_the_app_restarts() {
    let pipe = unique_pipe();
    let app = Arc::new(FakeApp::default());
    let stop = serve(&pipe, Arc::clone(&app));
    let mut host = Host::start(&pipe, &std::env::current_exe().unwrap());
    assert_eq!(host.send(ping(1)).await["type"], "pong");

    // The app quits (its connections close) and comes back.
    stop.cancel();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _stop = serve(&pipe, Arc::clone(&app));

    assert_eq!(host.send(ping(2)).await["type"], "pong");
    assert_eq!(app.requests.load(Ordering::SeqCst), 2);
    host.finish().await;
}
