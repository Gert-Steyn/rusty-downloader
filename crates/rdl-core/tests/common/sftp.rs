//! A small SFTP server for the tests: one file at `/srv/file.bin` (also as
//! `file.bin` in the home folder), a password login (`me` / `s3cret`) or the
//! user key [`USER_KEY`], and switches to change its host key, cut a
//! transfer off part-way, or go slowly. The keys are throwaway test keys.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::keys::{PublicKey, decode_secret_key};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::net::TcpListener;

pub const PATH: &str = "/srv/file.bin";
pub const HOME_FILE: &str = "file.bin";
pub const USER: &str = "me";
pub const PASSWORD: &str = "s3cret";
/// Modification time the server reports.
pub const MTIME: u32 = 1_793_000_000;

const HOST_KEYS: [&str; 2] = [
    "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACB92kkmCCwQ4fWk/zeLN/tbZocydMV56hv0NPp2I5qNZgAAAIhHk/97R5P/
ewAAAAtzc2gtZWQyNTUxOQAAACB92kkmCCwQ4fWk/zeLN/tbZocydMV56hv0NPp2I5qNZg
AAAEBLaonRW4JU0g/A3J7Y62N5WrsgGYVvEzXoEhRG2Wv+4n3aSSYILBDh9aT/N4s3+1tm
hzJ0xXnqG/Q0+nYjmo1mAAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----
",
    "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCI8JGumdqGAyP1wDoTu5o8oKcklugppC4/DA/HD3EU9gAAAIgvgnnNL4J5
zQAAAAtzc2gtZWQyNTUxOQAAACCI8JGumdqGAyP1wDoTu5o8oKcklugppC4/DA/HD3EU9g
AAAEBKR4o7an3bUeOXr5jDmvStGUGCw33F/Er1ySwoDv3JHojwka6Z2oYDI/XAOhO7mjyg
pySW6CmkLj8MD8cPcRT2AAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----
",
];

/// The private key the tests put in a key folder as `id_ed25519`.
pub const USER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACBN05fEPE3rJeUutSKc/VOY3K+hWZ6gTCGBMr+6ihoQbwAAAIj0jKm79Iyp
uwAAAAtzc2gtZWQyNTUxOQAAACBN05fEPE3rJeUutSKc/VOY3K+hWZ6gTCGBMr+6ihoQbw
AAAEBKS+VsawGJpvZ94xERoNGNn52Auj2SgnaW68ZA+pS0NU3Tl8Q8Tesl5S61Ipz9U5jc
r6FZnqBMIYEyv7qKGhBvAAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----
";
const USER_PUBLIC_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIE3Tl8Q8Tesl5S61Ipz9U5jcr6FZnqBMIYEyv7qKGhBv";

pub struct SftpServer {
    pub port: u16,
    pub data: Vec<u8>,
    /// Which of the two host keys new connections are shown.
    pub host_key: AtomicUsize,
    /// Transfers to cut off half-way (one each).
    pub drops_left: AtomicUsize,
    /// Pause on every read, so a test can stop a transfer part-way.
    pub slow: AtomicBool,
    /// `(user, "password" | "key")` for every login.
    pub logins: Mutex<Vec<(String, &'static str)>>,
    /// The offset each opened file was first read from.
    pub starts: Mutex<Vec<u64>>,
    pub bytes_served: AtomicU64,
}

impl SftpServer {
    pub fn url(&self, user: &str, password: Option<&str>, path: &str) -> url::Url {
        let password = password.map(|p| format!(":{p}")).unwrap_or_default();
        url::Url::parse(&format!(
            "sftp://{user}{password}@127.0.0.1:{}{path}",
            self.port
        ))
        .unwrap()
    }

    pub fn logins(&self) -> Vec<(String, &'static str)> {
        self.logins.lock().unwrap().clone()
    }

    pub fn starts(&self) -> Vec<u64> {
        self.starts.lock().unwrap().clone()
    }

    fn attrs(&self, path: &str) -> Option<FileAttributes> {
        let file = |size, permissions| FileAttributes {
            size: Some(size),
            permissions: Some(permissions),
            mtime: Some(MTIME),
            ..FileAttributes::default()
        };
        match path {
            PATH | HOME_FILE => Some(file(self.data.len() as u64, 0o100644)),
            "/srv" | "/srv/" => Some(file(0, 0o040755)),
            _ => None,
        }
    }
}

pub async fn spawn_sftp(size: usize) -> Arc<SftpServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = Arc::new(SftpServer {
        port: listener.local_addr().unwrap().port(),
        data: (0..size).map(|i| (i % 253) as u8).collect(),
        host_key: AtomicUsize::new(0),
        drops_left: AtomicUsize::new(0),
        slow: AtomicBool::new(false),
        logins: Mutex::default(),
        starts: Mutex::default(),
        bytes_served: AtomicU64::new(0),
    });
    let shared = Arc::clone(&server);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let key = HOST_KEYS[shared.host_key.load(Ordering::SeqCst)];
            let config = Arc::new(server::Config {
                keys: vec![decode_secret_key(key, None).unwrap()],
                auth_rejection_time: Duration::ZERO,
                auth_rejection_time_initial: Some(Duration::ZERO),
                ..server::Config::default()
            });
            let ssh = Ssh {
                server: Arc::clone(&shared),
                channels: HashMap::new(),
            };
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, stream, ssh).await {
                    let _ = session.await;
                }
            });
        }
    });
    server
}

struct Ssh {
    server: Arc<SftpServer>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Handler for Ssh {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if user == USER && password == PASSWORD {
            self.server
                .logins
                .lock()
                .unwrap()
                .push((user.into(), "password"));
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let known = PublicKey::from_openssh(USER_PUBLIC_KEY).unwrap();
        if user == USER && key.public_key.key_data() == known.key_data() {
            self.server
                .logins
                .lock()
                .unwrap()
                .push((user.into(), "key"));
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        match self.channels.remove(&channel_id) {
            Some(channel) if name == "sftp" => {
                session.channel_success(channel_id)?;
                let files = Files {
                    server: Arc::clone(&self.server),
                    open: HashMap::new(),
                    next: 0,
                };
                russh_sftp::server::run(channel.into_stream(), files).await;
            }
            _ => session.channel_failure(channel_id)?,
        }
        Ok(())
    }
}

struct Files {
    server: Arc<SftpServer>,
    /// Open handles, and whether each has been read from yet.
    open: HashMap<String, bool>,
    next: u32,
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

impl russh_sftp::server::Handler for Files {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let attrs = self.server.attrs(&path).ok_or(StatusCode::NoSuchFile)?;
        Ok(Attrs { id, attrs })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        if !self.open.contains_key(&handle) {
            return Err(StatusCode::Failure);
        }
        self.stat(id, PATH.into()).await
    }

    async fn realpath(&mut self, id: u32, _path: String) -> Result<Name, Self::Error> {
        Ok(Name {
            id,
            files: vec![File::dummy("/home/me")],
        })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        _flags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        match self.server.attrs(&filename) {
            Some(attrs) if attrs.is_regular() => {
                self.next += 1;
                let handle = format!("h{}", self.next);
                self.open.insert(handle.clone(), false);
                Ok(Handle { id, handle })
            }
            Some(_) => Err(StatusCode::Failure),
            None => Err(StatusCode::NoSuchFile),
        }
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let server = &self.server;
        let read_before = self.open.get_mut(&handle).ok_or(StatusCode::Failure)?;
        if !*read_before {
            *read_before = true;
            server.starts.lock().unwrap().push(offset);
        }
        let size = server.data.len() as u64;
        if offset >= size {
            return Err(StatusCode::Eof);
        }
        if offset >= size / 2 && server.drops_left.load(Ordering::SeqCst) > 0 {
            server.drops_left.fetch_sub(1, Ordering::SeqCst);
            self.open.remove(&handle);
            return Err(StatusCode::ConnectionLost);
        }
        if server.slow.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let end = (offset + len as u64).min(size);
        let data = server.data[offset as usize..end as usize].to_vec();
        server
            .bytes_served
            .fetch_add(data.len() as u64, Ordering::SeqCst);
        Ok(Data { id, data })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.open.remove(&handle);
        Ok(ok(id))
    }
}
