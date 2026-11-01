//! Tiny SOCKS5 and HTTP proxies for the tests: just enough of RFC 1928 /
//! 1929 and of HTTP `CONNECT` / absolute-form requests, with an optional
//! login, counting the connections they carry.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};

pub struct TestProxy {
    pub port: u16,
    /// Connections carried to a target.
    tunnels: AtomicUsize,
    /// Connections turned away for a wrong or missing login.
    refused: AtomicUsize,
    login: Option<(String, String)>,
}

impl TestProxy {
    pub fn tunnels(&self) -> usize {
        self.tunnels.load(Ordering::SeqCst)
    }

    pub fn refused(&self) -> usize {
        self.refused.load(Ordering::SeqCst)
    }

    fn accepts(&self, user: &str, pass: &str) -> bool {
        self.login
            .as_ref()
            .is_none_or(|(u, p)| u == user && p == pass)
    }
}

async fn listen(login: Option<(&str, &str)>) -> (TcpListener, Arc<TestProxy>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = Arc::new(TestProxy {
        port: listener.local_addr().unwrap().port(),
        tunnels: AtomicUsize::new(0),
        refused: AtomicUsize::new(0),
        login: login.map(|(u, p)| (u.to_owned(), p.to_owned())),
    });
    (listener, proxy)
}

/// A SOCKS5 proxy on 127.0.0.1; with `login`, only that user and password.
pub async fn socks5(login: Option<(&str, &str)>) -> Arc<TestProxy> {
    let (listener, proxy) = listen(login).await;
    let state = Arc::clone(&proxy);
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                let _ = socks5_session(client, &state).await;
            });
        }
    });
    proxy
}

async fn socks5_session(mut c: TcpStream, state: &TestProxy) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    c.read_exact(&mut head).await?;
    let mut methods = vec![0u8; head[1] as usize];
    c.read_exact(&mut methods).await?;
    if state.login.is_some() {
        if !methods.contains(&2) {
            state.refused.fetch_add(1, Ordering::SeqCst);
            return c.write_all(&[5, 0xff]).await;
        }
        c.write_all(&[5, 2]).await?;
        // RFC 1929: ver, ulen, user, plen, pass.
        let mut ver_len = [0u8; 2];
        c.read_exact(&mut ver_len).await?;
        let mut user = vec![0u8; ver_len[1] as usize];
        c.read_exact(&mut user).await?;
        let mut plen = [0u8; 1];
        c.read_exact(&mut plen).await?;
        let mut pass = vec![0u8; plen[0] as usize];
        c.read_exact(&mut pass).await?;
        let ok = state.accepts(
            &String::from_utf8_lossy(&user),
            &String::from_utf8_lossy(&pass),
        );
        c.write_all(&[1, if ok { 0 } else { 1 }]).await?;
        if !ok {
            state.refused.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
    } else {
        c.write_all(&[5, 0]).await?;
    }
    // Request: ver, cmd, rsv, atyp, addr, port.
    let mut req = [0u8; 4];
    c.read_exact(&mut req).await?;
    let host = match req[3] {
        1 => {
            let mut ip = [0u8; 4];
            c.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            c.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            c.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        _ => return c.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await,
    };
    let mut port = [0u8; 2];
    c.read_exact(&mut port).await?;
    let target = TcpStream::connect((host.as_str(), u16::from_be_bytes(port))).await;
    let Ok(mut target) = target else {
        return c.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
    };
    c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
    state.tunnels.fetch_add(1, Ordering::SeqCst);
    copy_bidirectional(&mut c, &mut target).await?;
    Ok(())
}

/// An HTTP proxy on 127.0.0.1; with `login`, only that user and password
/// (Basic `Proxy-Authorization`).
pub async fn http(login: Option<(&str, &str)>) -> Arc<TestProxy> {
    let (listener, proxy) = listen(login).await;
    let state = Arc::clone(&proxy);
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                let _ = http_session(client, &state).await;
            });
        }
    });
    proxy
}

async fn http_session(mut c: TcpStream, state: &TestProxy) -> std::io::Result<()> {
    // Read the request head.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if c.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let request_line = text.lines().next().unwrap_or_default().to_owned();
    let authorised = state.login.is_none()
        || text.lines().any(|line| {
            line.to_ascii_lowercase()
                .starts_with("proxy-authorization: basic ")
                && basic_login(&line[27..]).is_some_and(|(u, p)| state.accepts(&u, &p))
        });
    if !authorised {
        state.refused.fetch_add(1, Ordering::SeqCst);
        return c
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method == "CONNECT" {
        let mut target = TcpStream::connect(target).await?;
        c.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await?;
        state.tunnels.fetch_add(1, Ordering::SeqCst);
        copy_bidirectional(&mut c, &mut target).await?;
        return Ok(());
    }
    // Absolute form (`GET http://host:port/path`): the test server takes it as is.
    let authority = target
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_owned();
    let mut target = TcpStream::connect(authority.as_str()).await?;
    state.tunnels.fetch_add(1, Ordering::SeqCst);
    target.write_all(&head).await?;
    copy_bidirectional(&mut c, &mut target).await?;
    Ok(())
}

/// `user:pass` from a Basic credential.
fn basic_login(encoded: &str) -> Option<(String, String)> {
    let decoded = base64_decode(encoded.trim())?;
    let text = String::from_utf8(decoded).ok()?;
    let (u, p) = text.split_once(':')?;
    Some((u.to_owned(), p.to_owned()))
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|&c| c != b'=') {
        buf = (buf << 6) | ALPHABET.iter().position(|&a| a == c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}
