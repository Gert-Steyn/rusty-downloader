//! `rdl-native-host`: the browser starts this when the extension connects
//! (native messaging), and it relays each message to Rusty Downloader over
//! its named pipe, then the reply back. See `rdl-bridge` for the messages.
//!
//! stdout carries the protocol, so diagnostics go to stderr only. The host
//! runs until the browser closes stdin.

#[cfg(windows)]
mod launch;
#[cfg(windows)]
mod relay;
#[cfg(windows)]
mod trust;

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("rdl-native-host: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rdl-native-host: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
async fn run() -> std::io::Result<()> {
    use rdl_bridge::frame::{read_frame, write_frame};

    let mut relay = relay::Relay::new(relay::Config::from_env()?);
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    while let Some(message) = read_frame(&mut stdin).await? {
        let reply = relay.handle(&message).await;
        write_frame(&mut stdout, &reply).await?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("rdl-native-host only runs on Windows");
    ExitCode::FAILURE
}
