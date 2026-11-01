//! A small FTP server for the tests: one file at `/pub/file.bin`, a login
//! (`me` / `s3cret`) or anonymous access, passive mode, `SIZE`, `MDTM`,
//! `REST`, and switches to misbehave (drop a transfer part-way, refuse
//! `REST`, go slowly).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

pub const PATH: &str = "pub/file.bin";
pub const USER: &str = "me";
pub const PASSWORD: &str = "s3cret";
const CHUNK: usize = 16 * 1024;

pub struct FtpServer {
    pub port: u16,
    pub data: Vec<u8>,
    /// Let `anonymous` in.
    pub anonymous: AtomicBool,
    /// Answer `REST` with `502`.
    pub no_rest: AtomicBool,
    /// Transfers to cut off after this many bytes (one each).
    pub drops_left: AtomicUsize,
    /// Pause after every chunk, so a test can stop a transfer part-way.
    pub slow: AtomicBool,
    /// User names that logged in, and offsets `REST` asked for.
    pub logins: Mutex<Vec<String>>,
    pub rests: Mutex<Vec<u64>>,
    pub bytes_served: AtomicU64,
}

impl FtpServer {
    pub fn url(&self, login: Option<(&str, &str)>, path: &str) -> url::Url {
        let login = login.map(|(u, p)| format!("{u}:{p}@")).unwrap_or_default();
        url::Url::parse(&format!("ftp://{login}127.0.0.1:{}/{path}", self.port)).unwrap()
    }

    pub fn logins(&self) -> Vec<String> {
        self.logins.lock().unwrap().clone()
    }

    pub fn rests(&self) -> Vec<u64> {
        self.rests.lock().unwrap().clone()
    }
}

pub async fn spawn_ftp(size: usize) -> Arc<FtpServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = Arc::new(FtpServer {
        port: listener.local_addr().unwrap().port(),
        data: (0..size).map(|i| (i % 251) as u8).collect(),
        anonymous: AtomicBool::new(true),
        no_rest: AtomicBool::new(false),
        drops_left: AtomicUsize::new(0),
        slow: AtomicBool::new(false),
        logins: Mutex::default(),
        rests: Mutex::default(),
        bytes_served: AtomicU64::new(0),
    });
    let shared = Arc::clone(&server);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let server = Arc::clone(&shared);
            tokio::spawn(async move {
                let _ = session(server, stream).await;
            });
        }
    });
    server
}

async fn session(server: Arc<FtpServer>, stream: TcpStream) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"220 test server ready\r\n").await?;
    let mut user = String::new();
    let mut logged_in = false;
    let mut passive: Option<TcpListener> = None;
    let mut offset = 0u64;
    while let Some(line) = lines.next_line().await? {
        let (command, arg) = line.split_once(' ').unwrap_or((&line, ""));
        let reply = match command.to_ascii_uppercase().as_str() {
            "USER" => {
                user = arg.to_owned();
                "331 password please".to_owned()
            }
            "PASS" => {
                let anonymous = user == "anonymous" && server.anonymous.load(Ordering::SeqCst);
                if anonymous || (user == USER && arg == PASSWORD) {
                    logged_in = true;
                    server.logins.lock().unwrap().push(user.clone());
                    "230 logged in".to_owned()
                } else {
                    "530 Login incorrect.".to_owned()
                }
            }
            "QUIT" => {
                write.write_all(b"221 bye\r\n").await?;
                return Ok(());
            }
            _ if !logged_in => "530 Please log in.".to_owned(),
            "TYPE" => "200 type set".to_owned(),
            "SIZE" if arg == PATH => format!("213 {}", server.data.len()),
            "MDTM" if arg == PATH => "213 20261101120000".to_owned(),
            "SIZE" | "MDTM" => "550 No such file.".to_owned(),
            "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let port = listener.local_addr()?.port();
                passive = Some(listener);
                format!(
                    "227 Entering Passive Mode (127,0,0,1,{},{})",
                    port >> 8,
                    port & 0xff
                )
            }
            "EPSV" => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let port = listener.local_addr()?.port();
                passive = Some(listener);
                format!("229 Entering Extended Passive Mode (|||{port}|)")
            }
            "REST" if server.no_rest.load(Ordering::SeqCst) => {
                "502 REST not implemented".to_owned()
            }
            "REST" => {
                offset = arg.parse().unwrap_or(0);
                server.rests.lock().unwrap().push(offset);
                format!("350 restarting at {offset}")
            }
            "RETR" if arg != PATH => "550 No such file.".to_owned(),
            "RETR" => {
                let Some(listener) = passive.take() else {
                    write.write_all(b"425 use PASV first\r\n").await?;
                    continue;
                };
                write.write_all(b"150 opening data connection\r\n").await?;
                let (mut data, _) = listener.accept().await?;
                let start = std::mem::take(&mut offset) as usize;
                let cut = server.drops_left.load(Ordering::SeqCst) > 0;
                if cut {
                    server.drops_left.fetch_sub(1, Ordering::SeqCst);
                }
                let end = if cut {
                    (start + (server.data.len() - start) / 2).max(start + 1)
                } else {
                    server.data.len()
                };
                for chunk in server.data[start.min(end)..end].chunks(CHUNK) {
                    if data.write_all(chunk).await.is_err() {
                        return Ok(()); // the client went away
                    }
                    server
                        .bytes_served
                        .fetch_add(chunk.len() as u64, Ordering::SeqCst);
                    if server.slow.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
                if cut {
                    // Cut off: the data connection and the session just end.
                    return Ok(());
                }
                data.shutdown().await?;
                drop(data);
                "226 transfer complete".to_owned()
            }
            _ => "502 not implemented".to_owned(),
        };
        write.write_all(format!("{reply}\r\n").as_bytes()).await?;
    }
    Ok(())
}
