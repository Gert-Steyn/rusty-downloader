//! Core of rusty-downloader: everything that is not UI. The Tauri app (and later the
//! native-messaging host and remote server) depend on this crate; it never
//! depends on Tauri.

pub mod db;
pub mod error;
pub mod http;
pub mod logging;
pub mod manager;
pub mod paths;

pub use error::{Error, Result};

/// Crate version, shared by the app shell.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
