use serde::Serialize;
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::state::SharedState;
use crate::window_labels;
use crate::windows::{self, ChatTarget, SettingsTab};

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

/// Open a chat window: 1:1 with a friend or community member, or a group
/// DM. The window title is resolved from backend state.
#[tauri::command]
pub async fn open_chat_window(
    target: ChatTarget,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    windows::open_chat_window(&app, state.inner(), &pool, &target).await
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

/// Open an https link in the system browser (`IpcRequest::LinkOpen`): the
/// Host holds it for review, an `OpenExternalUrl` whose proof states that
/// the site will see our IP address, and once committed answers the
/// normalized URL, which this opens. This is the only way the webview opens
/// a link: the opener plugin's own link interception is off.
#[tauri::command]
pub async fn open_external_url(
    url: String,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::state::SharedState>,
) -> Result<rekindle_types::dto::Done<String>, rekindle_types::dto::CommandError> {
    let request = rekindle_ipc::protocol::IpcRequest::LinkOpen { url };
    let done: rekindle_types::dto::Done<String> =
        match crate::host_call::call(&state, request).await {
            Ok(done) => done,
            Err(rekindle_types::dto::CommandError::IntentRequired(review)) => {
                // Its commit opens the URL (`commands::intent::intent_commit`).
                state.pending_link_opens.lock().insert(review.intent_id);
                return Err(rekindle_types::dto::CommandError::IntentRequired(review));
            }
            Err(e) => return Err(e),
        };
    window
        .app_handle()
        .opener()
        .open_url(done.value.as_str(), None::<&str>)
        .map_err(|e| rekindle_types::dto::CommandError::internal(e.to_string()))?;
    Ok(done)
}

/// Get the current Veilid network status.
#[tauri::command]
pub async fn get_network_status(state: State<'_, SharedState>) -> Result<NetworkStatus, String> {
    Ok(crate::services::window_runtime::get_network_status_inner(
        state.inner(),
    ))
}
