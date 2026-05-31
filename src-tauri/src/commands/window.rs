use serde::Serialize;
use tauri::{Manager, State};

use crate::state::SharedState;
use crate::windows;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatus {
    pub attachment_state: String,
    pub is_attached: bool,
    pub public_internet_ready: bool,
    pub has_route: bool,
    pub profile_dht_key: Option<String>,
    pub friend_list_dht_key: Option<String>,
}

/// Transition from login window to buddy list after successful authentication.
///
/// Creates a fresh buddy-list window so its `onMount → hydrateState()` runs
/// AFTER the backend identity is set, then destroys the login window.
#[tauri::command]
pub async fn show_buddy_list(app: tauri::AppHandle) -> Result<(), String> {
    windows::open_buddy_list(&app)?;
    // Destroy login window (if still alive). Using destroy() for immediate
    // label cleanup — close() is async and would cause label collisions
    // if the user somehow triggers this path again quickly.
    if let Some(login) = app.get_webview_window("login") {
        let _ = login.destroy();
    }
    Ok(())
}

/// Open a chat window for a 1:1 conversation.
#[tauri::command]
pub async fn open_chat_window(
    public_key: String,
    display_name: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    windows::open_chat_window(&app, &public_key, &display_name)
}

/// Open a DM window for a SMPL-record-backed direct message
/// (architecture §27).
#[tauri::command]
pub async fn open_dm_window(
    record_key: String,
    title_hint: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    windows::open_dm_window(&app, &record_key, &title_hint)
}

/// Open the settings window, optionally to a specific tab.
#[tauri::command]
pub async fn open_settings_window(
    tab: Option<String>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    windows::open_settings(&app, tab.as_deref())
}

/// Open a community window.
#[tauri::command]
pub async fn open_community_window(
    community_id: String,
    community_name: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    windows::open_community_window(&app, &community_id, &community_name)
}

/// Open a profile window for viewing a friend's profile.
#[tauri::command]
pub async fn open_profile_window(
    public_key: String,
    display_name: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    windows::open_profile_window(&app, &public_key, &display_name)
}

/// Wave 12 W12.7 — pop the active call into its own webview window.
#[tauri::command]
pub async fn open_call_window(
    call_id: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    windows::open_call_window(&app, &call_id)
}

/// Get the current Veilid network status.
#[tauri::command]
pub async fn get_network_status(state: State<'_, SharedState>) -> Result<NetworkStatus, String> {
    Ok(crate::services::window_runtime::get_network_status_inner(state.inner()))
}
