//! Settings → Network: proxy profiles, the rules that pick them, and a
//! test that fetches a page through one.

use rdl_core::proxy::{ProxyChoice, ProxyInput, ProxyProfile, ProxySettings};
use serde::Serialize;
use specta::Type;
use tauri::State;
use url::Url;

use crate::commands::CommandResult;
use crate::state::AppState;

/// Windows' own connectivity check: tiny, plain HTTP, and always up.
const TEST_URL: &str = "http://www.msftconnecttest.com/connecttest.txt";

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProxyOverview {
    pub profiles: Vec<ProxyProfile>,
    pub settings: ProxySettings,
}

#[tauri::command]
#[specta::specta]
pub async fn proxy_overview(state: State<'_, AppState>) -> CommandResult<ProxyOverview> {
    Ok(ProxyOverview {
        profiles: state.network.profiles(),
        settings: state.network.settings(),
    })
}

/// A new profile; returns its id.
#[tauri::command]
#[specta::specta]
pub async fn proxy_add(state: State<'_, AppState>, input: ProxyInput) -> CommandResult<i64> {
    Ok(state.network.add_profile(input).await?)
}

/// `input.password`: `null` keeps the stored one, `""` removes it.
#[tauri::command]
#[specta::specta]
pub async fn proxy_update(
    state: State<'_, AppState>,
    id: i64,
    input: ProxyInput,
) -> CommandResult<()> {
    Ok(state.network.update_profile(id, input).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn proxy_remove(state: State<'_, AppState>, id: i64) -> CommandResult<()> {
    Ok(state.network.remove_profile(id).await?)
}

/// The default proxy and the rules, saved as a whole.
#[tauri::command]
#[specta::specta]
pub async fn proxy_set_settings(
    state: State<'_, AppState>,
    settings: ProxySettings,
) -> CommandResult<()> {
    Ok(state.network.set_settings(settings).await?)
}

/// Fetch a small page through `choice` (by default Windows' connectivity
/// check); returns how long the answer took in milliseconds.
#[tauri::command]
#[specta::specta]
pub async fn proxy_test(
    state: State<'_, AppState>,
    choice: ProxyChoice,
    url: Option<String>,
) -> CommandResult<u32> {
    let raw = url.filter(|u| !u.trim().is_empty());
    let url = Url::parse(raw.as_deref().unwrap_or(TEST_URL).trim())
        .map_err(|e| format!("That isn't a link: {e}"))?;
    let took = state.network.test(&url, choice).await?;
    Ok(took.as_millis().min(u128::from(u32::MAX)) as u32)
}
