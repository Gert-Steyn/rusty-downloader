//! Network adapters, for binding every connection to one of them (a VPN's)
//! and noticing when it goes down. An adapter whose only addresses are
//! link-local (`169.254.x.x`, `fe80::`) counts as down: Windows gives those
//! to adapters that aren't connected, Tailscale's among them.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// An adapter as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    /// Windows' id for it (a GUID), stable across renames and reboots.
    pub id: String,
    /// The name shown in Windows' settings ("Wi-Fi", "Tailscale").
    pub name: String,
    /// The driver's description ("WireGuard Tunnel").
    pub description: String,
    /// The address connections would be sent from; `None` while it's down.
    pub address: Option<IpAddr>,
}

/// The adapter chosen in the settings: its id, and its name for when it's gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct BoundAdapter {
    pub id: String,
    pub name: String,
}

/// The address to bind to among an adapter's: IPv4 first (what VPNs route
/// by default), else a global IPv6; never link-local or loopback.
pub fn usable_address(addresses: &[IpAddr]) -> Option<IpAddr> {
    let usable = |ip: &&IpAddr| match ip {
        IpAddr::V4(v4) => !v4.is_link_local() && !v4.is_loopback() && !v4.is_unspecified(),
        IpAddr::V6(v6) => {
            // fe80::/10 is link-local.
            !v6.is_loopback() && !v6.is_unspecified() && (v6.segments()[0] & 0xffc0) != 0xfe80
        }
    };
    addresses
        .iter()
        .filter(usable)
        .find(|ip| ip.is_ipv4())
        .or_else(|| addresses.iter().find(|ip| usable(ip)))
        .copied()
}

/// Every adapter except loopback, in Windows' order. Failing to ask
/// Windows is logged and gives an empty list.
#[cfg(windows)]
pub fn list() -> Vec<AdapterInfo> {
    use ipconfig::{IfType, OperStatus};
    match ipconfig::get_adapters() {
        Ok(adapters) => adapters
            .iter()
            .filter(|a| a.if_type() != IfType::SoftwareLoopback)
            .map(|a| AdapterInfo {
                id: a.adapter_name().to_owned(),
                name: a.friendly_name().to_owned(),
                description: a.description().to_owned(),
                address: (a.oper_status() == OperStatus::IfOperStatusUp)
                    .then(|| usable_address(a.ip_addresses()))
                    .flatten(),
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list network adapters");
            Vec::new()
        }
    }
}

#[cfg(not(windows))]
pub fn list() -> Vec<AdapterInfo> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ips(list: &[&str]) -> Vec<IpAddr> {
        list.iter().map(|s| s.parse().unwrap()).collect()
    }

    #[test]
    fn picks_ipv4_then_global_ipv6_never_link_local() {
        assert_eq!(
            usable_address(&ips(&["fe80::1", "2001:db8::5", "10.64.0.2"])),
            Some("10.64.0.2".parse().unwrap())
        );
        assert_eq!(
            usable_address(&ips(&["fe80::1", "2001:db8::5", "169.254.83.107"])),
            Some("2001:db8::5".parse().unwrap())
        );
        assert_eq!(usable_address(&ips(&["fe80::1", "169.254.83.107"])), None);
        assert_eq!(usable_address(&ips(&["127.0.0.1", "::1"])), None);
        assert_eq!(usable_address(&[]), None);
    }

    /// Asks Windows for real; there is always at least one adapter.
    #[cfg(windows)]
    #[test]
    fn lists_this_machines_adapters() {
        let adapters = list();
        assert!(!adapters.is_empty());
        assert!(
            adapters
                .iter()
                .all(|a| !a.id.is_empty() && !a.name.is_empty())
        );
    }
}
