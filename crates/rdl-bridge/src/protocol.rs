//! The messages, version 1.
//!
//! Every message is a JSON object with the protocol version `v`, an
//! optional `id` the reply echoes (so a client can match replies to
//! requests), and a `type`:
//!
//! | Request | Reply |
//! |---|---|
//! | `ping` | `pong` with the app's name, version and protocol |
//! | `status` | `status` with active and queued downloads and LinkGrabber links |
//! | `add` | `added` with what was added, skipped and rejected |
//!
//! Anything else gets an `error` reply. Unknown fields are ignored, so the
//! extension can send more than a given app version reads (a browser
//! cookie has several fields nobody here needs).

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// Most links one `add` may carry ("all links on this page" can be many).
pub const MAX_LINKS: usize = 1000;

/// The envelope around every request and reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message<T> {
    pub v: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(flatten)]
    pub body: T,
}

impl<T> Message<T> {
    pub fn new(id: Option<u64>, body: T) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Request {
    Ping,
    Status,
    Add(AddRequest),
}

/// Where added links go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    /// The LinkGrabber, to be checked and confirmed by the user.
    #[default]
    Grabber,
    /// Straight into the download queue (a download the browser was about
    /// to save itself).
    Queue,
}

/// Links, plus what the browser would send with them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddRequest {
    pub links: Vec<String>,
    #[serde(default)]
    pub target: Target,
    /// The page the links are on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// The browser's cookies for the links' sites.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cookies: Vec<BrowserCookie>,
    /// File name the browser picked, for a single link sent to the queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Package name; by default the app picks one from the links.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
}

/// A cookie as `chrome.cookies.getAll` returns it (Firefox's `browser.cookies`
/// has the same fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default)]
    pub host_only: bool,
    #[serde(default = "root_path")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub session: bool,
    /// Seconds since the Unix epoch, with a fraction; absent for session
    /// cookies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<f64>,
}

fn root_path() -> String {
    "/".into()
}

impl BrowserCookie {
    /// Expiry in whole seconds; `None` for a session cookie.
    pub fn expires(&self) -> Option<i64> {
        if self.session {
            return None;
        }
        self.expiration_date
            .filter(|e| e.is_finite())
            .map(|e| e.floor() as i64)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Reply {
    Pong {
        app: String,
        version: String,
        protocol: u32,
    },
    Status {
        active: u32,
        queued: u32,
        grabber: u32,
    },
    Added {
        target: Target,
        added: u32,
        /// Already in the LinkGrabber.
        duplicates: u32,
        /// Not http(s) links.
        rejected: Vec<String>,
    },
    Error {
        message: String,
        /// Set when the extension can act on the kind of failure.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<ErrorCode>,
    },
}

/// Failures the extension tells apart. These come from the native host,
/// which answers for the app when it can't reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// The app isn't running (and the request didn't start it).
    NotRunning,
    /// Something other than the app holds its pipe; nothing was sent to it.
    Untrusted,
    /// The app is running but didn't answer properly.
    Unavailable,
}

impl Reply {
    pub fn error(message: impl Into<String>) -> Self {
        Reply::Error {
            message: message.into(),
            code: None,
        }
    }

    pub fn error_with(code: ErrorCode, message: impl Into<String>) -> Self {
        Reply::Error {
            message: message.into(),
            code: Some(code),
        }
    }
}

/// Read a request. A message that isn't one is answered with an error
/// reply, carrying its `id` when that much could be read.
pub fn decode_request(bytes: &[u8]) -> Result<Message<Request>, Message<Reply>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| Message::new(None, Reply::error(format!("not JSON: {e}"))))?;
    let id = value.get("id").and_then(serde_json::Value::as_u64);
    let fail = |message: String| Message::new(id, Reply::error(message));
    match value.get("v").and_then(serde_json::Value::as_u64) {
        Some(v) if v == u64::from(PROTOCOL_VERSION) => {}
        Some(v) => {
            return Err(fail(format!(
                "protocol version {v} is not supported (this app speaks {PROTOCOL_VERSION})"
            )));
        }
        None => return Err(fail("missing protocol version \"v\"".into())),
    }
    let message: Message<Request> =
        serde_json::from_value(value).map_err(|e| fail(format!("bad request: {e}")))?;
    if let Request::Add(add) = &message.body {
        if add.links.is_empty() {
            return Err(fail("no links to add".into()));
        }
        if add.links.len() > MAX_LINKS {
            return Err(fail(format!(
                "{} links in one message (at most {MAX_LINKS})",
                add.links.len()
            )));
        }
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(json: &str) -> Result<Message<Request>, Message<Reply>> {
        decode_request(json.as_bytes())
    }

    fn error_of(json: &str) -> (Option<u64>, String) {
        match decode(json).unwrap_err() {
            Message {
                id,
                body: Reply::Error { message, .. },
                ..
            } => (id, message),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn reads_requests() {
        assert_eq!(
            decode(r#"{"v":1,"id":7,"type":"ping"}"#).unwrap(),
            Message::new(Some(7), Request::Ping)
        );
        assert_eq!(
            decode(r#"{"v":1,"type":"status","extra":true}"#)
                .unwrap()
                .body,
            Request::Status
        );
        let add = decode(
            r#"{"v":1,"type":"add","links":["https://example.com/a.zip"],
                "referer":"https://example.com/","userAgent":"UA",
                "cookies":[{"name":"sid","value":"1","domain":".example.com","hostOnly":false,
                            "path":"/","secure":true,"httpOnly":true,"sameSite":"lax",
                            "session":false,"expirationDate":1893456000.25,"storeId":"0"}]}"#,
        )
        .unwrap();
        let Request::Add(add) = add.body else {
            panic!("expected add")
        };
        assert_eq!(add.target, Target::Grabber);
        assert_eq!(add.user_agent.as_deref(), Some("UA"));
        assert_eq!(add.cookies[0].expires(), Some(1_893_456_000));
        assert!(add.cookies[0].http_only);
    }

    #[test]
    fn session_cookies_have_no_expiry() {
        let cookie: BrowserCookie = serde_json::from_str(
            r#"{"name":"a","value":"b","domain":"x.test","session":true,"expirationDate":5}"#,
        )
        .unwrap();
        assert_eq!(cookie.expires(), None);
        assert_eq!(cookie.path, "/");
    }

    #[test]
    fn refuses_what_it_cannot_serve() {
        assert!(error_of("nope").1.starts_with("not JSON"));
        assert_eq!(
            error_of(r#"{"id":3,"type":"ping"}"#),
            (Some(3), "missing protocol version \"v\"".into())
        );
        assert!(
            error_of(r#"{"v":2,"id":4,"type":"ping"}"#)
                .1
                .contains("version 2 is not supported")
        );
        let (id, message) = error_of(r#"{"v":1,"id":5,"type":"delete"}"#);
        assert_eq!(id, Some(5));
        assert!(message.contains("unknown variant"), "{message}");
        assert!(error_of(r#"{"v":1,"type":"add"}"#).1.contains("links"));
        assert_eq!(
            error_of(r#"{"v":1,"type":"add","links":[]}"#).1,
            "no links to add"
        );
        let many = format!(
            r#"{{"v":1,"type":"add","links":[{}]}}"#,
            vec![r#""https://x.test/""#; MAX_LINKS + 1].join(",")
        );
        assert!(error_of(&many).1.contains("at most"));
    }

    #[test]
    fn replies_are_flat_objects() {
        let reply = Message::new(
            Some(1),
            Reply::Added {
                target: Target::Queue,
                added: 2,
                duplicates: 0,
                rejected: vec!["ftp://x".into()],
            },
        );
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"v":1,"id":1,"type":"added","target":"queue","added":2,"duplicates":0,"rejected":["ftp://x"]}"#
        );
        let pong = Message::new(
            None,
            Reply::Pong {
                app: "rusty-downloader".into(),
                version: "0.1.0".into(),
                protocol: 1,
            },
        );
        assert_eq!(
            serde_json::to_string(&pong).unwrap(),
            r#"{"v":1,"type":"pong","app":"rusty-downloader","version":"0.1.0","protocol":1}"#
        );
    }
}
