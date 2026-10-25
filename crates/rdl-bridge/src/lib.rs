//! The bridge between rusty-downloader and the browser.
//!
//! The browser extension talks to `rdl-native-host` over native messaging;
//! the host relays each message over a named pipe to the running app, which
//! answers on the same pipe. This crate holds what both ends share: the
//! messages ([`protocol`]), their framing ([`frame`]) and the pipe itself
//! ([`pipe`]). It doesn't depend on the engine, so the host stays small.
//!
//! Framing is the same as native messaging (a little-endian `u32` length,
//! then that many bytes of UTF-8 JSON), so the host can pass messages
//! through without re-encoding them. The pipe is local, but anything the
//! user runs can connect to it, so every message is untrusted input.

pub mod frame;
#[cfg(windows)]
pub mod pipe;
pub mod protocol;

pub use protocol::{
    AddRequest, BrowserCookie, ErrorCode, MAX_LINKS, Message, PROTOCOL_VERSION, Reply, Request,
    Target,
};
