//! Every webview window the app opens.
//!
//! All windows go through [`base_builder`], which applies the classic Xfire
//! frameless chrome and the security policy in [`navigation`]. Window labels
//! come from [`crate::window_labels`], per-entity windows take validated ids
//! from `rekindle_types::key_format`, and titles are resolved in [`titles`].
//! Nothing the webview passes in reaches a label, URL or title unchecked.

mod first_show;
mod navigation;
pub mod popup;
pub mod popup_place;
mod rebind;
mod reveal;
mod shape;
mod titles;

pub use rebind::follow_event;
pub use reveal::reveal_window;

use rekindle_types::key_format::{self, PublicKeyHex};
use serde::{Deserialize, Serialize};
use tauri::utils::config::BackgroundThrottlingPolicy;
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
    Appearance,
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
            Self::Appearance => "appearance",
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
        WindowKind::Chat => ((480.0, 550.0), (380.0, 400.0)),
        WindowKind::Community => ((900.0, 650.0), (750.0, 500.0)),
        WindowKind::Call => ((420.0, 540.0), (360.0, 420.0)),
        WindowKind::Profile => ((380.0, 500.0), (340.0, 420.0)),
        WindowKind::SkinEditor => ((640.0, 620.0), (480.0, 480.0)),
        // Placed and sized from the served popup (`popup::open`).
        WindowKind::SkinPopup => ((200.0, 300.0), (100.0, 100.0)),
    };
    Size { inner, min }
}

/// The builder every window starts from: frameless transparent chrome,
/// the navigation guard, no new windows, the media-permission policy for
/// `kind`, background throttling, and the data root's webview store.
///
/// No OS shadow (plan SK0 S2/S3, research §5): the window is cut to its
/// skin's silhouette ([`shape`]), and a shadow is drawn around the window
/// **rectangle**, not the silhouette. On Windows `shadow(true)` also adds a
/// 1px white border and Windows 11's rounded corners (tauri 2.12.1
/// `window/mod.rs:2124-2132`), which a region window cannot have anyway;
/// Linux does not support it; on macOS AppKit's shape-derived shadow is
/// unreliable over WKWebView's layer tree. Classic Xfire drew none.
///
/// Background throttling is `Throttle` (plan A1.5, finding E7.14 F5): a
/// hidden or covered window limits its timers but keeps running, so the
/// buddy list in the tray and a covered chat window still take events,
/// countdowns and badges. Without a policy, macOS WKWebView's inactive
/// scheduling suspends a hidden view's tasks entirely; Signal and Element
/// Desktop keep Electron's throttle-not-suspend default. wry maps it to
/// WKPreferences `inactiveSchedulingPolicy` (macOS 14+, the E7.10 floor);
/// WebKitGTK and WebView2 ignore it. `cargo xtask
/// check-background-throttling` holds every window to this builder.
///
/// # Errors
/// No data root is managed yet (`setup::run` manages it before the first
/// window): a window is never built on the platform's default store when
/// the root names another.
fn base_builder<'a>(
    app: &'a AppHandle,
    kind: WindowKind,
    label: &str,
    path: String,
) -> Result<WebviewWindowBuilder<'a, Wry, AppHandle>, String> {
    let store = app
        .try_state::<rekindle_db::paths::DataRoot>()
        .map(|root| root.webview.clone())
        .ok_or("no data root is managed")?;
    let dev_origin = if tauri::is_dev() {
        app.config().build.dev_url.clone()
    } else {
        None
    };
    let captures_media = kind.captures_media();
    let Size { inner, min } = size(kind);
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(path.into()))
        .inner_size(inner.0, inner.1)
        .min_inner_size(min.0, min.1)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(true)
        .background_throttling(BackgroundThrottlingPolicy::Throttle)
        .on_navigation(move |url| navigation::is_app_origin(url, dev_origin.as_ref()))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_permission_request(move |_, permission| {
            navigation::permission_for(captures_media, permission)
        });
    Ok(with_store(builder, &store))
}

/// Point the webview at `store`: a real install keeps the platform's
/// defaults; a contained root (the WebDriver walk) gets its own data
/// directory, which WebKitGTK and WebView2 keep every store in, and on
/// macOS, where WKWebView ignores it, a non-persistent store.
fn with_store<'a>(
    builder: WebviewWindowBuilder<'a, Wry, AppHandle>,
    store: &rekindle_db::paths::WebviewStore,
) -> WebviewWindowBuilder<'a, Wry, AppHandle> {
    match store {
        rekindle_db::paths::WebviewStore::Platform(_) => builder,
        rekindle_db::paths::WebviewStore::Contained(dir) => {
            let builder = builder.data_directory(dir.clone());
            #[cfg(target_os = "macos")]
            let builder = builder.incognito(true);
            builder
        }
    }
}

/// Build the window hidden, enable platform media capture if `kind`
/// captures, then show it ([`first_show`]: a window built visible opens
/// taller than its inner size on GTK Wayland), and shape it once shown
/// ([`shape::attach`]; GTK realizes the GDK window on the show).
fn finish(
    builder: WebviewWindowBuilder<'_, Wry, AppHandle>,
    kind: WindowKind,
) -> Result<WebviewWindow, String> {
    let window = first_show::open(builder, |window| {
        if kind.captures_media() {
            crate::platform::enable_webview_media_capture(window);
        }
    })?;
    shape::attach(&window, shape::frame_of(kind));
    Ok(window)
}

/// Show, restore and focus an existing window ([`reveal_window`]). Returns
/// false when `label` is not open.
fn raise(app: &AppHandle, label: &str) -> bool {
    let Some(window) = app.get_webview_window(label) else {
        return false;
    };
    reveal_window(&window);
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
    let builder = base_builder(app, WindowKind::Login, labels::LOGIN, path)?
        .title("Rekindle")
        .center();
    finish(builder, WindowKind::Login).map(drop)
}

/// Open the buddy list window (the main window after login), or bring the
/// open one back (the tray, the walk's recovery).
///
/// An open buddy list belongs to this session (a session's end destroys
/// every window but the login, `shell_session::ended::show_login`), so it is
/// revealed in place, never rebuilt: destroying a tray-hidden window and
/// building another under its label left it hidden on COSMIC. A new one
/// hydrates after the backend identity is set.
pub fn open_buddy_list(app: &AppHandle) -> Result<(), String> {
    if raise(app, labels::BUDDY_LIST) {
        return Ok(());
    }
    let builder = base_builder(
        app,
        WindowKind::BuddyList,
        labels::BUDDY_LIST,
        "/buddy-list".to_owned(),
    )?
    .title("Rekindle")
    .max_inner_size(400.0, 900.0);
    finish(builder, WindowKind::BuddyList).map(drop)
}

/// What a chat window shows: a 1:1 conversation with a friend (identity
/// key) or a community member (pseudonym key), or a group DM by its group
/// id (32 hex, plan E2.6).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatTarget {
    Peer(String),
    Group(String),
}

/// Open (or raise) the chat window for `target`.
pub async fn open_chat_window(
    app: &AppHandle,
    state: &SharedState,
    pool: &Db,
    target: &ChatTarget,
) -> Result<(), String> {
    let (label, path, title, selection) = match target {
        ChatTarget::Peer(peer) => {
            let peer = key_format::public_key_hex(peer).map_err(|e| invalid("peer key", &e))?;
            let label = labels::chat_label(&peer);
            if raise(app, &label) {
                return Ok(());
            }
            let title = titles::chat(state, pool, &peer).await;
            // The window's skin selection is its own peer (lead decision 48).
            let selection = rekindle_types::dto::skin_run::SkinSelection::Friend {
                public_key: peer.as_str().to_owned(),
            };
            (label, format!("/chat?peer={peer}"), title, Some(selection))
        }
        ChatTarget::Group(group) => {
            let group = labels::GroupIdHex::parse(group)
                .ok_or_else(|| "invalid group id: not 32 lowercase hex".to_owned())?;
            let label = labels::group_chat_label(&group);
            if raise(app, &label) {
                return Ok(());
            }
            (
                label,
                format!("/chat?group={}", group.as_str()),
                titles::GROUP.to_owned(),
                None,
            )
        }
    };
    let builder = base_builder(app, WindowKind::Chat, &label, path)?.title(title);
    finish(builder, WindowKind::Chat)?;
    crate::skin_windows::opened(app, &label, selection);
    Ok(())
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
        reveal_window(&window);
        return Ok(());
    }
    let path = match tab {
        Some(tab) => format!("/settings?tab={}", tab.as_str()),
        None => "/settings".to_owned(),
    };
    let builder = base_builder(app, WindowKind::Settings, labels::SETTINGS, path)?
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
    let builder = base_builder(app, WindowKind::Community, &label, path)?
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
    if let Err(e) = window.request_user_attention(Some(tauri::UserAttentionType::Critical)) {
        tracing::trace!(error = %e, "surface_buddy_list: user_attention failed");
    }
    reveal_window(&window);
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
        base_builder(app, WindowKind::Call, &label, format!("/call?id={call}"))?.title("Call");
    finish(builder, WindowKind::Call).map(drop)
}

/// The 1:1 call is over: destroy its window (plan CL2).
///
/// `open_call_window` makes one window per call (its label carries the
/// call id), so a window left open after the call ended was never reused:
/// every call added a webview whose compositor GL contexts and WebKit IPC
/// connections stayed alive until the user closed it (Linux, 5 calls:
/// +2–3 Mesa GL contexts and +1–2 `IPC::Connection` receive queues per
/// call, all still present 2 min after the last hang-up). `destroy`, not
/// `hide`: a hidden webview keeps every one of those resources, and
/// `destroy` skips the `CloseRequested` round trip (tauri 2
/// `WebviewWindow::destroy`), which for a call window has nothing to
/// intercept. No window for the call (a group call, or one that never
/// connected) is a no-op.
pub fn retire_call_window(app: &AppHandle, call: &str) {
    let Some(label) = labels::call_label_for(call) else {
        return;
    };
    let Some(window) = app.get_webview_window(&label) else {
        return;
    };
    if let Err(e) = window.destroy() {
        tracing::warn!(error = %e, "could not destroy the ended call's window");
    }
}

/// Open (or raise) the skin editor (plan E7.7.5: a Settings sub-window,
/// one at a time). Its draft's live style applies to its own document
/// only; the commands that read and export a draft are granted to it alone
/// (`capabilities/app-skin-editor.json`).
pub fn open_skin_editor(app: &AppHandle) -> Result<(), String> {
    if raise(app, labels::SKIN_EDITOR) {
        return Ok(());
    }
    let mut builder = base_builder(
        app,
        WindowKind::SkinEditor,
        labels::SKIN_EDITOR,
        "/skin-editor".to_owned(),
    )?
    .title("Rekindle - Skin Editor");
    if let Some(settings) = app.get_webview_window(labels::SETTINGS) {
        builder = builder.parent(&settings).map_err(|e| e.to_string())?;
    }
    finish(builder, WindowKind::SkinEditor).map(drop)
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
    )?
    .title(title);
    finish(builder, WindowKind::Profile).map(drop)
}
