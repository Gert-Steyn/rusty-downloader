//! Settings → Network: proxy profiles, the rules that pick them, and a
//! test that fetches a page through one; and Settings → Accounts.

use rdl_core::accounts::{Account, AccountInput};
use rdl_core::network::CookieImport;
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

/// Site accounts, without passwords or cookie values.
#[tauri::command]
#[specta::specta]
pub async fn account_list(state: State<'_, AppState>) -> CommandResult<Vec<Account>> {
    Ok(state.network.accounts())
}

/// A new account; returns its id.
#[tauri::command]
#[specta::specta]
pub async fn account_add(state: State<'_, AppState>, input: AccountInput) -> CommandResult<i64> {
    Ok(state.network.add_account(input).await?)
}

/// `input.password`: `null` keeps the stored one, `""` removes it.
#[tauri::command]
#[specta::specta]
pub async fn account_update(
    state: State<'_, AppState>,
    id: i64,
    input: AccountInput,
) -> CommandResult<()> {
    Ok(state.network.update_account(id, input).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn account_remove(state: State<'_, AppState>, id: i64) -> CommandResult<()> {
    Ok(state.network.remove_account(id).await?)
}

/// Add the cookies for the account's site from a Netscape `cookies.txt`.
#[tauri::command]
#[specta::specta]
pub async fn account_import_cookies(
    state: State<'_, AppState>,
    id: i64,
    path: String,
) -> CommandResult<CookieImport> {
    const MAX: u64 = 16 * 1024 * 1024;
    let size = tokio::fs::metadata(&path)
        .await
        .map_err(|e| format!("Can't read {path}: {e}"))?
        .len();
    if size > MAX {
        return Err("That file is too big for a cookies.txt.".to_owned().into());
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Can't read {path}: {e}"))?;
    Ok(state
        .network
        .import_cookies(id, &String::from_utf8_lossy(&bytes))
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn account_clear_cookies(state: State<'_, AppState>, id: i64) -> CommandResult<()> {
    Ok(state.network.clear_cookies(id).await?)
}
