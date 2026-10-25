//! The pipe server against real named pipes, with a fake handler. Each test
//! uses its own pipe name.
#![cfg(windows)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rdl_bridge::frame::{read_frame, write_frame};
use rdl_bridge::pipe::{self, Handler, Server};
use rdl_bridge::{Message, Reply, Request, Target};
use tokio_util::sync::CancellationToken;

/// Pongs, and "adds" every link it's given, counting how many adds run at once.
#[derive(Default)]
struct Fake {
    in_flight: AtomicUsize,
    peak: AtomicUsize,
}

impl Handler for Fake {
    async fn handle(&self, request: Request) -> Reply {
        match request {
            Request::Ping => Reply::Pong {
                app: "fake".into(),
                version: "0".into(),
                protocol: 1,
            },
            Request::Status => Reply::Status {
                active: 1,
                queued: 2,
                grabber: 3,
            },
            Request::Add(add) => {
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(now, Ordering::SeqCst);
                // Slow enough that concurrent clients overlap.
                tokio::time::sleep(Duration::from_millis(200)).await;
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                Reply::Added {
                    target: add.target,
                    added: add.links.len() as u32,
                    duplicates: 0,
                    rejected: Vec::new(),
                }
            }
        }
    }
}

fn unique_name() -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    format!(
        r"\\.\pipe\rdl-bridge-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

/// A served pipe; dropping it stops the server.
struct Running {
    name: String,
    fake: Arc<Fake>,
    stop: CancellationToken,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn serve() -> Running {
    let name = unique_name();
    let server = Server::bind(&name).unwrap();
    let stop = CancellationToken::new();
    let fake = Arc::new(Fake::default());
    tokio::spawn(server.run(Arc::clone(&fake), stop.clone()));
    Running { name, fake, stop }
}

fn add(links: &[&str]) -> Message<Request> {
    Message::new(
        Some(9),
        Request::Add(rdl_bridge::AddRequest {
            links: links.iter().map(|l| l.to_string()).collect(),
            target: Target::Queue,
            referer: None,
            user_agent: None,
            cookies: Vec::new(),
            filename: None,
            package: None,
        }),
    )
}

#[tokio::test]
async fn answers_several_requests_on_one_connection() {
    let running = serve();
    let mut client = pipe::connect(&running.name).await.unwrap();

    let pong = pipe::call(&mut client, &Message::new(Some(1), Request::Ping))
        .await
        .unwrap();
    assert_eq!(pong.id, Some(1));
    assert!(matches!(pong.body, Reply::Pong { protocol: 1, .. }));

    let status = pipe::call(&mut client, &Message::new(None, Request::Status))
        .await
        .unwrap();
    assert_eq!(status.id, None);
    assert_eq!(
        status.body,
        Reply::Status {
            active: 1,
            queued: 2,
            grabber: 3
        }
    );

    let added = pipe::call(&mut client, &add(&["https://x.test/a", "https://x.test/b"]))
        .await
        .unwrap();
    assert_eq!(added.id, Some(9));
    assert!(matches!(
        added.body,
        Reply::Added {
            target: Target::Queue,
            added: 2,
            ..
        }
    ));
}

#[tokio::test]
async fn bad_messages_get_an_error_and_the_connection_stays_usable() {
    let running = serve();
    let mut client = pipe::connect(&running.name).await.unwrap();
    for (raw, wanted) in [
        (
            &br#"{"v":1,"id":4,"type":"format-disk"}"#[..],
            "unknown variant",
        ),
        (br#"{"v":9,"id":4,"type":"ping"}"#, "version 9"),
        (b"\xff\xfe not json", "not JSON"),
    ] {
        let reply = pipe::exchange(&mut client, raw).await.unwrap();
        let reply: Message<Reply> = serde_json::from_slice(&reply).unwrap();
        let Reply::Error { message, .. } = reply.body else {
            panic!("expected an error, got {reply:?}")
        };
        assert!(message.contains(wanted), "{message}");
    }
    let pong = pipe::call(&mut client, &Message::new(None, Request::Ping))
        .await
        .unwrap();
    assert!(matches!(pong.body, Reply::Pong { .. }));
}

#[tokio::test]
async fn an_oversized_message_is_refused_and_the_connection_closed() {
    let running = serve();
    let mut client = pipe::connect(&running.name).await.unwrap();
    // Only the length is sent: the server must not wait for 4 GiB.
    use tokio::io::AsyncWriteExt;
    client.write_all(&u32::MAX.to_le_bytes()).await.unwrap();
    let reply = read_frame(&mut client).await.unwrap().unwrap();
    let reply: Message<Reply> = serde_json::from_slice(&reply).unwrap();
    assert!(matches!(reply.body, Reply::Error { .. }));
    assert_eq!(read_frame(&mut client).await.ok().flatten(), None);
}

#[tokio::test]
async fn serves_clients_at_the_same_time() {
    let running = serve();
    let calls = (0..6).map(|_| {
        let name = running.name.clone();
        tokio::spawn(async move {
            let mut client = pipe::connect(&name).await.unwrap();
            pipe::call(&mut client, &add(&["https://x.test/a"]))
                .await
                .unwrap()
        })
    });
    for call in calls.collect::<Vec<_>>() {
        assert!(matches!(
            call.await.unwrap().body,
            Reply::Added { added: 1, .. }
        ));
    }
    assert!(
        running.fake.peak.load(Ordering::SeqCst) > 1,
        "requests ran one by one"
    );
}

#[tokio::test]
async fn the_name_cannot_be_taken_twice() {
    let running = serve();
    let err = Server::bind(&running.name)
        .err()
        .expect("second bind fails");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
}

#[tokio::test]
async fn nobody_answers_when_the_app_is_not_running() {
    let err = pipe::connect(&unique_name()).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);

    // Nor after it stopped.
    let running = serve();
    let name = running.name.clone();
    drop(running);
    tokio::time::sleep(Duration::from_millis(100)).await;
    match pipe::connect(&name).await {
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
        // The listening instance may linger until the runtime drops it;
        // it must not answer, though.
        Ok(mut client) => {
            let _ = write_frame(&mut client, br#"{"v":1,"type":"ping"}"#).await;
            let reply =
                tokio::time::timeout(Duration::from_millis(300), read_frame(&mut client)).await;
            assert!(
                !matches!(reply, Ok(Ok(Some(_)))),
                "a stopped server replied"
            );
        }
    }
}
