//! Every webview window the app opens.
//!
//! All windows go through [`base_builder`], which applies the classic Xfire
//! frameless chrome and the security policy in [`navigation`]. Window labels
//! come from [`crate::window_labels`], per-entity windows take validated ids
//! from `rekindle_types::key_format`, and titles are resolved in [`titles`].
//! Nothing the webview passes in reaches a label, URL or title unchecked.

mod navigation;
mod titles;

use rekindle_types::key_format::{self, PublicKeyHex};
use serde::{Deserialize, Serialize};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};

use crate::state::SharedState;
use crate::window_labels::{self as labels, WindowKind};
use rekindle_db::Db;

/// A settings tab, as named by `src/windows/SettingsWindow.tsx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsTab {
    Profile,
    Application,
    Notifications,
    Audio,
    Video,
    Privacy,
    Devices,
    Mobile,
    About,
}

impl SettingsTab {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Application => "application",
            Self::Notifications => "notifications",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Privacy => "privacy",
            Self::Devices => "devices",
            Self::Mobile => "mobile",
            Self::About => "about",
        }
    }
}

/// Size limits for one window kind: (inner, min inner).
struct Size {
    inner: (f64, f64),
    min: (f64, f64),
}

const fn size(kind: WindowKind) -> Size {
    let (inner, min) = match kind {
        WindowKind::Login => ((380.0, 480.0), (340.0, 440.0)),
        WindowKind::BuddyList => ((320.0, 650.0), (300.0, 500.0)),
        WindowKind::Settings => ((500.0, 550.0), (420.0, 450.0)),
        WindowKind::Chat | WindowKind::Dm => ((480.0, 550.0), (380.0, 400.0)),
        WindowKind::Community => ((900.0, 650.0), (750.0, 500.0)),
        WindowKind::Call => ((420.0, 540.0), (360.0, 420.0)),
        WindowKind::Profile => ((380.0, 500.0), (340.0, 420.0)),
    };
    Size { inner, min }
}

/// The builder every window starts from: frameless transparent chrome,
/// the navigation guard, no new windows, and the media-permission policy
/// for `kind`.
fn base_builder<'a>(
    app: &'a AppHandle,
    kind: WindowKind,
    label: &str,
    path: String,
) -> WebviewWindowBuilder<'a, Wry, AppHandle> {
    let dev_origin = if tauri::is_dev() {
        app.config().build.dev_url.clone()
    } else {
        None
    };
    let captures_media = kind.captures_media();
    let Size { inner, min } = size(kind);
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(path.into()))
        .inner_size(inner.0, inner.1)
        .min_inner_size(min.0, min.1)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .on_navigation(move |url| navigation::is_app_origin(url, dev_origin.as_ref()))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_permission_request(move |_, permission| {
            navigation::permission_for(captures_media, permission)
        })
}

/// Build the window, then enable platform media capture if `kind` captures.
fn finish(
    builder: WebviewWindowBuilder<'_, Wry, AppHandle>,
    kind: WindowKind,
) -> Result<WebviewWindow, String> {
    let window = builder.build().map_err(|e| e.to_string())?;
    if kind.captures_media() {
        crate::platform::enable_webview_media_capture(&window);
    }
    Ok(window)
}

/// Show and focus an existing window. Returns false when `label` is not open.
fn raise(app: &AppHandle, label: &str) -> bool {
    let Some(window) = app.get_webview_window(label) else {
        return false;
    };
    let _ = window.show();
    let _ = window.set_focus();
    true
}

fn invalid(what: &str, e: &key_format::KeyFormatError) -> String {
    format!("invalid {what}: {e}")
}

/// Open (or re-open) the login window.
///
/// When `preselect` is given (after logout), the login window opens on the
/// passphrase screen for that account instead of the account picker.
///
/// An existing login window is destroyed first: its URL cannot change, and
/// `destroy()` (unlike the async `close()`) frees the label immediately.
pub fn open_login(app: &AppHandle, preselect: Option<&PublicKeyHex>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(labels::LOGIN) {
        let _ = window.destroy();
    }
    let path = match preselect {
        Some(key) => format!("/login?account={key}"),
        None => "/login".to_owned(),
    };
    let builder = base_builder(app, WindowKind::Login, labels::LOGIN, path)
        .title("Rekindle")
        .center();
    finish(builder, WindowKind::Login).map(drop)
}

/// Open the buddy list window (the main window after login).
///
/// An existing buddy list is destroyed first so the new one hydrates after
/// the backend identity is set.
pub fn open_buddy_list(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(labels::BUDDY_LIST) {
        let _ = window.destroy();
    }
    let builder = base_builder(
        app,
        WindowKind::BuddyList,
        labels::BUDDY_LIST,
        "/buddy-list".to_owned(),
    )
    .title("Rekindle")
    .max_inner_size(400.0, 900.0);
    finish(builder, WindowKind::BuddyList).map(drop)
}

/// Open a 1:1 chat window with a friend (identity key) or a community member
/// (pseudonym key).
pub async fn open_chat_window(
    app: &AppHandle,
    state: &SharedState,
    pool: &Db,
    peer: &str,
) -> Result<(), String> {
    let peer = key_format::public_key_hex(peer).map_err(|e| invalid("peer key", &e))?;
    let label = labels::chat_label(&peer);
    if raise(app, &label) {
        return Ok(());
    }
    let title = titles::chat(state, pool, &peer).await;
    let builder =
        base_builder(app, WindowKind::Chat, &label, format!("/chat?peer={peer}")).title(title);
    finish(builder, WindowKind::Chat).map(drop)
}

/// Open the window for a SMPL-record-backed DM conversation (architecture
/// §27), keyed by its record key.
pub async fn open_dm_window(
    app: &AppHandle,
    state: &SharedState,
    pool: &Db,
    record: &str,
) -> Result<(), String> {
    let record = key_format::record_key(record).map_err(|e| invalid("DM record key", &e))?;
    let label = labels::dm_label(&record);
    if raise(app, &label) {
        return Ok(());
    }
    let title = titles::dm(state, pool, &record).await;
    let builder =
        base_builder(app, WindowKind::Dm, &label, format!("/dm?record={record}")).title(title);
    finish(builder, WindowKind::Dm).map(drop)
}

/// Open the settings window (single instance), optionally on `tab`.
///
/// An open settings window switches tab through `settings-switch-tab`
/// instead of being rebuilt, so unsaved edits survive.
pub fn open_settings(app: &AppHandle, tab: Option<SettingsTab>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(labels::SETTINGS) {
        if let Some(tab) = tab {
            crate::event_dispatch::emit(
                app,
                crate::event_dispatch::WebviewEvent::SettingsSwitchTab(tab),
            );
        }
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let path = match tab {
        Some(tab) => format!("/settings?tab={}", tab.as_str()),
        None => "/settings".to_owned(),
    };
    let builder = base_builder(app, WindowKind::Settings, labels::SETTINGS, path)
        .title("Rekindle - Settings");
    finish(builder, WindowKind::Settings).map(drop)
}

/// Open a community window, or the community browser when `community` is
/// `None`.
pub fn open_community_window(
    app: &AppHandle,
    state: &SharedState,
    community: Option<&str>,
) -> Result<(), String> {
    let community = community
        .map(key_format::record_key)
        .transpose()
        .map_err(|e| invalid("community id", &e))?;
    let label = labels::community_label(community.as_ref());
    if raise(app, &label) {
        return Ok(());
    }
    let path = match &community {
        Some(id) => format!("/community?id={id}"),
        None => "/community".to_owned(),
    };
    let builder = base_builder(app, WindowKind::Community, &label, path)
        .title(titles::community(state, community.as_ref()));
    finish(builder, WindowKind::Community).map(drop)
}

/// Bring the buddy list forward for something the user must answer now:
/// a ringing call, a session-reset request.
///
/// Shows and focuses the buddy list and requests critical user attention:
/// the dock icon bounces (macOS), the taskbar entry flashes (Windows), or
/// the window manager sets the urgency hint (Linux). Mirrors how
/// Discord/Signal/Telegram make a ringing call unmistakable. Failures are
/// logged and never abort the caller.
pub fn surface_buddy_list(app: &AppHandle) {
    let Some(window) = app.get_webview_window(labels::BUDDY_LIST) else {
        tracing::debug!("surface_buddy_list: buddy-list window not found");
        return;
    };
    if let Err(e) = window.show() {
        tracing::warn!(error = %e, "surface_buddy_list: show failed");
    }
    if let Err(e) = window.unminimize() {
        tracing::trace!(error = %e, "surface_buddy_list: unminimize failed");
    }
    if let Err(e) = window.set_focus() {
        tracing::warn!(error = %e, "surface_buddy_list: set_focus failed");
    }
    if let Err(e) = window.request_user_attention(Some(tauri::UserAttentionType::Critical)) {
        tracing::trace!(error = %e, "surface_buddy_list: user_attention failed");
    }
}

/// Wave 12 W12.7 — pop the active call out into its own window. It mounts
/// the same `<CallController />` (see `main.tsx`) on the `/call` route, and
/// renders from the same `callsState` as every other window.
pub fn open_call_window(app: &AppHandle, call: &str) -> Result<(), String> {
    let call = key_format::call_id(call).map_err(|e| invalid("call id", &e))?;
    let label = labels::call_label(&call);
    if raise(app, &label) {
        return Ok(());
    }
    let builder =
        base_builder(app, WindowKind::Call, &label, format!("/call?id={call}")).title("Call");
    finish(builder, WindowKind::Call).map(drop)
}

/// Open a profile window for a friend or community member.
pub async fn open_profile_window(
    app: &AppHandle,
    state: &SharedState,
    pool: &Db,
    peer: &str,
) -> Result<(), String> {
    let peer = key_format::public_key_hex(peer).map_err(|e| invalid("peer key", &e))?;
    let label = labels::profile_label(&peer);
    if raise(app, &label) {
        return Ok(());
    }
    let title = titles::profile(state, pool, &peer).await;
    let builder = base_builder(
        app,
        WindowKind::Profile,
        &label,
        format!("/profile?key={peer}"),
    )
    .title(title);
    finish(builder, WindowKind::Profile).map(drop)
}
