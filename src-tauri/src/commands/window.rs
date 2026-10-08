use serde::Serialize;
use tauri::{Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_opener::OpenerExt;

use crate::state::SharedState;
use crate::window_labels;
use crate::windows::{self, SettingsTab};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatus {
    pub attachment_state: String,
    pub is_attached: bool,
    pub public_internet_ready: bool,
    pub has_route: bool,
    /// What the media-class route is doing (plan C7.9c).
    pub media_route: rekindle_types::subscription_events::RouteAvailability,
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
    if let Some(login) = app.get_webview_window(window_labels::LOGIN) {
        let _ = login.destroy();
    }
    Ok(())
}

/// Open a chat window with a friend or community member. The window title
/// is resolved from backend state.
#[tauri::command]
pub async fn open_chat_window(
    public_key: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    windows::open_chat_window(&app, state.inner(), &pool, &public_key).await
}

/// Open a DM window for a SMPL-record-backed direct message
/// (architecture §27).
#[tauri::command]
pub async fn open_dm_window(
    record_key: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    windows::open_dm_window(&app, state.inner(), &pool, &record_key).await
}

/// Open the settings window, optionally to a specific tab.
#[tauri::command]
pub async fn open_settings_window(
    tab: Option<SettingsTab>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    windows::open_settings(&app, tab)
}

/// Open a community window, or the community browser when `community_id`
/// is absent.
#[tauri::command]
pub async fn open_community_window(
    community_id: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    windows::open_community_window(&app, state.inner(), community_id.as_deref())
}

/// Open a profile window for a friend or community member.
#[tauri::command]
pub async fn open_profile_window(
    public_key: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    windows::open_profile_window(&app, state.inner(), &pool, &public_key).await
}

/// Wave 12 W12.7 — pop the active call into its own webview window.
#[tauri::command]
pub async fn open_call_window(call_id: String, app: tauri::AppHandle) -> Result<(), String> {
    windows::open_call_window(&app, &call_id)
}

/// Open an https link in the system browser after a native confirmation
/// naming the host (in punycode for international names). Returns `false`
/// if the user declined. This is the only way the webview opens a link:
/// the opener plugin's own link interception is off.
#[tauri::command]
pub async fn open_external_url(url: String, window: tauri::WebviewWindow) -> Result<bool, String> {
    let url = rekindle_link_preview::https_url(&url).map_err(|e| e.to_string())?;
    let host = url.host_str().unwrap_or_default().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .message(format!(
            "Open {host} in your browser?\n\nThe site will see your IP address."
        ))
        .title("Open link")
        .buttons(MessageDialogButtons::OkCancel)
        .parent(&window)
        .show(move |confirmed| {
            let _ = tx.send(confirmed);
        });
    if !rx
        .await
        .map_err(|_| "link dialog closed unexpectedly".to_string())?
    {
        return Ok(false);
    }
    window
        .app_handle()
        .opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Get the current Veilid network status.
#[tauri::command]
pub async fn get_network_status(state: State<'_, SharedState>) -> Result<NetworkStatus, String> {
    Ok(crate::services::window_runtime::get_network_status_inner(
        state.inner(),
    ))
}
