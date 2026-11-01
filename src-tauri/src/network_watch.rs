//! Keeps every connection on the adapter chosen in Settings → Network (a
//! VPN's): bound to its address while it's up; while it's down nothing
//! connects, and with the kill-switch on downloads are held and carry on by
//! themselves when it's back. A new address (the VPN reconnected) restarts
//! running downloads on it. Checked every 2 s and right after the settings change.

use std::time::Duration;

use rdl_core::adapters::{self, AdapterInfo};
use rdl_core::manager::Manager as DownloadManager;
use rdl_core::network::{Binding, Network};
use rdl_core::settings::Settings;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event as _;

use crate::commands::CommandResult;
use crate::notify;
use crate::state::AppState;

const CHECK_EVERY: Duration = Duration::from_secs(2);

/// What Settings → Network and the status bar show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NetworkStatus {
    /// No adapter chosen: Windows routes connections.
    #[default]
    Any,
    Up {
        adapter: String,
        address: String,
    },
    /// `held`: downloads wait for it (kill-switch); otherwise they fail.
    Down {
        adapter: String,
        held: bool,
    },
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatusChanged {
    pub status: NetworkStatus,
}

/// What the settings and the adapters call for right now.
pub struct Wanted {
    pub binding: Binding,
    pub hold: Option<String>,
    pub status: NetworkStatus,
}

pub fn wanted(settings: &Settings, adapters: &[AdapterInfo]) -> Wanted {
    let Some(bound) = &settings.bind_adapter else {
        return Wanted {
            binding: Binding::Any,
            hold: None,
            status: NetworkStatus::Any,
        };
    };
    let found = adapters.iter().find(|a| a.id == bound.id);
    let name = found.map_or(bound.name.clone(), |a| a.name.clone());
    match found.and_then(|a| a.address) {
        Some(ip) => Wanted {
            binding: Binding::Bound(ip),
            hold: None,
            status: NetworkStatus::Up {
                adapter: name,
                address: ip.to_string(),
            },
        },
        None => Wanted {
            binding: Binding::Down(name.clone()),
            hold: settings.kill_switch.then(|| format!("{name} is down")),
            status: NetworkStatus::Down {
                adapter: name,
                held: settings.kill_switch,
            },
        },
    }
}

/// Put `wanted` into effect, in an order that never lets a connection out
/// unbound: hold before giving up the address, bind before releasing.
pub fn apply(network: &Network, downloads: &DownloadManager, wanted: &Wanted) {
    if wanted.hold.is_some() {
        downloads.set_network_hold(wanted.hold.clone());
    }
    let before = network.binding();
    if network.set_binding(wanted.binding.clone())
        && matches!(
            (&before, &wanted.binding),
            (Binding::Bound(_), Binding::Bound(_))
        )
    {
        // A new address: running downloads still use the old one.
        tracing::info!("bound adapter's address changed; restarting downloads");
        downloads.restart_running();
    }
    if wanted.hold.is_none() {
        downloads.set_network_hold(None);
    }
}

pub async fn watch(app: AppHandle) {
    let mut tick = tokio::time::interval(CHECK_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let Some(state) = app.try_state::<AppState>() else {
            tick.tick().await;
            continue;
        };
        tokio::select! {
            _ = tick.tick() => {}
            _ = state.network_check.notified() => {}
            _ = state.shutdown.cancelled() => break,
        }
        let settings = state.settings().clone();
        let adapters = tokio::task::spawn_blocking(adapters::list)
            .await
            .unwrap_or_default();
        let wanted = wanted(&settings, &adapters);
        apply(&state.network, &state.downloads, &wanted);
        let before = state.set_network_status(wanted.status.clone());
        if before != wanted.status {
            tracing::info!(status = ?wanted.status, "network status");
            tell_the_user(&app, &before, &wanted.status);
            let _ = NetworkStatusChanged {
                status: wanted.status,
            }
            .emit(&app);
        }
    }
}

/// A Windows notification when downloads stop or start again because of the adapter.
fn tell_the_user(app: &AppHandle, before: &NetworkStatus, now: &NetworkStatus) {
    match (before, now) {
        (
            NetworkStatus::Up { .. },
            NetworkStatus::Down {
                adapter,
                held: true,
            },
        ) => notify::message(
            app,
            format!("{adapter} is down"),
            "Downloads are paused and carry on when it's back.".into(),
        ),
        (
            NetworkStatus::Up { .. },
            NetworkStatus::Down {
                adapter,
                held: false,
            },
        ) => notify::message(
            app,
            format!("{adapter} is down"),
            "Downloads can't connect until it's back.".into(),
        ),
        (NetworkStatus::Down { held: true, .. }, NetworkStatus::Up { adapter, .. }) => {
            notify::message(
                app,
                format!("{adapter} is back"),
                "Downloads carry on.".into(),
            )
        }
        _ => {}
    }
}

/// Network adapters on this machine, for choosing one.
#[tauri::command]
#[specta::specta]
pub async fn network_adapters() -> CommandResult<Vec<AdapterInfo>> {
    Ok(tokio::task::spawn_blocking(adapters::list)
        .await
        .unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub async fn network_status(state: State<'_, AppState>) -> CommandResult<NetworkStatus> {
    Ok(state.network_status())
}

#[cfg(test)]
mod tests {
    use rdl_core::adapters::BoundAdapter;

    use super::*;

    fn adapter(address: Option<&str>) -> AdapterInfo {
        AdapterInfo {
            id: "{GUID}".into(),
            name: "Tailscale".into(),
            description: "Tailscale Tunnel".into(),
            address: address.map(|a| a.parse().unwrap()),
        }
    }

    fn settings(kill_switch: bool) -> Settings {
        Settings {
            bind_adapter: Some(BoundAdapter {
                id: "{GUID}".into(),
                name: "Old name".into(),
            }),
            kill_switch,
            ..Settings::default()
        }
    }

    #[test]
    fn nothing_chosen_is_any() {
        let w = wanted(&Settings::default(), &[adapter(None)]);
        assert_eq!(
            (w.binding, w.hold, w.status),
            (Binding::Any, None, NetworkStatus::Any)
        );
    }

    #[test]
    fn up_binds_to_its_address() {
        let w = wanted(&settings(true), &[adapter(Some("100.64.0.7"))]);
        assert_eq!(w.binding, Binding::Bound("100.64.0.7".parse().unwrap()));
        assert_eq!(w.hold, None);
        assert_eq!(
            w.status,
            NetworkStatus::Up {
                adapter: "Tailscale".into(),
                address: "100.64.0.7".into()
            }
        );
    }

    #[test]
    fn down_or_gone_blocks_and_holds_only_with_the_kill_switch() {
        let w = wanted(&settings(true), &[adapter(None)]);
        assert_eq!(w.binding, Binding::Down("Tailscale".into()));
        assert_eq!(w.hold.as_deref(), Some("Tailscale is down"));

        let w = wanted(&settings(false), &[adapter(None)]);
        assert_eq!(w.binding, Binding::Down("Tailscale".into()));
        assert_eq!(w.hold, None);

        // Gone altogether: known by its saved name.
        let w = wanted(&settings(true), &[]);
        assert_eq!(w.binding, Binding::Down("Old name".into()));
        assert_eq!(
            w.status,
            NetworkStatus::Down {
                adapter: "Old name".into(),
                held: true
            }
        );
    }
}
