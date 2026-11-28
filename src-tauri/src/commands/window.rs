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
    /// What the general-class route is doing (plan RV2).
    pub general_route: rekindle_types::subscription_events::RouteAvailability,
    /// What the media-class route is doing (plan C7.9c).
    pub media_route: rekindle_types::subscription_events::RouteAvailability,
    /// Whether the Host accepts writes now (its lifecycle's `can_write`;
    /// plan E7.13: a community create waits for it).
    pub can_write: bool,
    /// The indicator's bars as shown now (plan E7.2); later changes arrive
    /// as `NetworkEvent::Strength`.
    pub bars: u8,
    /// Where the own-records stage is (plan E2.5 FC3): an invite, a friend
    /// request or a first message waits for `held`; later changes arrive
    /// as `NetworkEvent::OwnRecordsChanged`.
    pub own_records: rekindle_types::subscription_events::OwnRecordsPhase,
    /// Why the stage failed, when it did.
    pub own_records_error: Option<String>,
    /// Why a route class is down with no new route (plan RV1), `null` when
    /// none is: the route owner's state the Host's `StatusSnapshot.route_paused`
    /// reads; later changes arrive as `NetworkEvent::RoutePaused` /
    /// `RouteResumed`.
    pub route_paused: Option<rekindle_types::subscription_events::RoutePauseCause>,
    /// Which own route classes are down, and since when (plan RV1g): the
    /// Host's `StatusSnapshot.own_route_down`; later changes arrive as
    /// `NetworkEvent::OwnRouteDown` / `OwnRouteUp`.
    pub own_route_down: rekindle_types::subscription_events::OwnRouteDownSet,
    pub profile_dht_key: Option<String>,
    pub friend_list_dht_key: Option<String>,
}

/// Transition from login window to buddy list after successful authentication.
///
/// Creates a fresh buddy-list window so its `onMount → hydrateState()` runs
/// AFTER the backend identity is set, then destroys the login window.
#[tauri::command]
pub async fn show_buddy_list(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    windows::open_buddy_list(&app, state.inner()).await?;
    // Destroy login window (if still alive). Using destroy() for immediate
    // label cleanup — close() is async and would cause label collisions
    // if the user somehow triggers this path again quickly.
    if let Some(login) = app.get_webview_window(window_labels::LOGIN) {
        let _ = login.destroy();
    }
    Ok(())
}

/// Quit Rekindle from the menu bar: the tray's Quit (`tray::quit_rekindle`). Closing
/// the buddy list only hides it and leaves the app in the tray.
#[tauri::command]
pub async fn quit_app(app: tauri::AppHandle) {
    crate::tray::quit_rekindle(&app);
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

/// Open the skin editor from Settings → Appearance (plan E7.7.5).
#[tauri::command]
pub async fn open_skin_editor_window(app: tauri::AppHandle) -> Result<(), String> {
    windows::open_skin_editor(&app)
}

/// Open, or close if open, the pack popup `popup` beside the calling skin
/// window (plan SK2-A5). The Host has answered the pack's `LaunchPopup`;
/// the popup's place comes from the skin the Host shows now (`SkinGet`),
/// never from the webview. An invalid name, or a skin whose launcher has
/// no such popup, is an error.
#[tauri::command]
pub async fn open_skin_popup(
    popup: String,
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let name = window_labels::PopupName::parse(&popup)
        .ok_or_else(|| "invalid popup name: 1 to 64 ASCII letters and digits".to_owned())?;
    let outline = windows::popup::launcher_outline(&window)?;
    if windows::popup::close_if_open(&app, &window, &name) {
        return Ok(());
    }
    let skin: rekindle_types::dto::Done<rekindle_types::dto::skin_library::SkinApplied> =
        crate::host_call::call(
            &state,
            rekindle_ipc::protocol::IpcRequest::SkinGet { id: None },
        )
        .await
        .map_err(|e| e.to_string())?;
    let place = windows::popup_place::served_popup(&skin.value.shells, outline, name.as_str())
        .ok_or_else(|| "the shown skin has no such popup for this window".to_owned())?;
    windows::popup::open(&app, state.inner(), &window, &name, place)
}

/// Open a community window, or the community browser when `community_id`
/// is absent.
#[tauri::command]
pub async fn open_community_window(
    community_id: Option<String>,
    channel: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    windows::open_community_window(
        &app,
        state.inner(),
        community_id.as_deref(),
        channel.as_deref(),
    )
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
