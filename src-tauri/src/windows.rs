use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Open (or re-show) the login window.
///
/// When `preselect_key` is provided (e.g. after logout), the login window
/// opens directly on the passphrase screen for that account instead of the
/// generic picker. The user can still hit "Back" to reach the picker.
pub fn open_login(app: &AppHandle, preselect_key: Option<&str>) -> Result<(), String> {
    // If the window already exists we can't change its URL, so destroy it first
    // to ensure a fresh load with the correct query param. Using destroy() instead
    // of close() because close is async and the label would still be registered
    // when we try to create the new window.
    if let Some(window) = app.get_webview_window("login") {
        let _ = window.destroy();
    }

    let path = match preselect_key {
        Some(key) => format!("/login?account={key}"),
        None => "/login".to_string(),
    };

    WebviewWindowBuilder::new(app, "login", WebviewUrl::App(path.into()))
        .title("Rekindle")
        .inner_size(380.0, 480.0)
        .min_inner_size(340.0, 440.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .center()
        .build()
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}

/// Open the buddy list window (main window after login).
///
/// Destroys any existing buddy-list window first to avoid label conflicts.
/// Uses `destroy()` instead of `close()` because close is async and the old
/// label would still be registered when we try to create the new window.
pub fn open_buddy_list(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("buddy-list") {
        let _ = window.destroy();
    }

    WebviewWindowBuilder::new(app, "buddy-list", WebviewUrl::App("/buddy-list".into()))
        .title("Rekindle")
        .inner_size(320.0, 650.0)
        .min_inner_size(300.0, 500.0)
        .max_inner_size(400.0, 900.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .build()
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}

/// Open a chat window for a 1:1 conversation.
pub fn open_chat_window(
    app: &AppHandle,
    public_key: &str,
    display_name: &str,
) -> Result<(), String> {
    let label = format!("chat-{}", &public_key[..16.min(public_key.len())]);

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let url = WebviewUrl::App(format!("/chat?peer={public_key}").into());
    WebviewWindowBuilder::new(app, &label, url)
        .title(format!("Chat - {display_name}"))
        .inner_size(480.0, 550.0)
        .min_inner_size(380.0, 400.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .build()
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}

/// Open a DM window for a SMPL-record-backed direct message
/// conversation (architecture §27). Each DM gets its own window, keyed
/// by the truncated record key — distinct from `open_chat_window`,
/// which is for legacy 1:1 friend chats over Signal Protocol.
pub fn open_dm_window(app: &AppHandle, record_key: &str, title_hint: &str) -> Result<(), String> {
    let suffix: String = record_key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(20)
        .collect();
    let label = format!("dm-{suffix}");

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let url = WebviewUrl::App(format!("/dm?record={record_key}").into());
    WebviewWindowBuilder::new(app, &label, url)
        .title(format!("DM - {title_hint}"))
        .inner_size(480.0, 550.0)
        .min_inner_size(380.0, 400.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .build()
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}

/// Open the settings window (single instance), optionally to a specific tab.
///
/// If the window already exists, emits a `settings-switch-tab` event to change
/// the active tab without destroying the window (preserving unsaved state).
/// If the window doesn't exist, creates it with the tab as a query parameter.
pub fn open_settings(app: &AppHandle, tab: Option<&str>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        if let Some(t) = tab {
            let _ = window.emit("settings-switch-tab", t);
        }
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let path = match tab {
        Some(t) => format!("/settings?tab={t}"),
        None => "/settings".to_string(),
    };

    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App(path.into()))
        .title("Rekindle - Settings")
        .inner_size(500.0, 550.0)
        .min_inner_size(420.0, 450.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .build()
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}

/// Open a community window (per-community, matches community-* capability pattern).
pub fn open_community_window(
    app: &AppHandle,
    community_id: &str,
    community_name: &str,
) -> Result<(), String> {
    let id_part = if community_id.is_empty() {
        "browser"
    } else {
        &community_id[..16.min(community_id.len())]
    };
    let label = format!("community-{id_part}");

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let url = WebviewUrl::App(format!("/community?id={community_id}").into());
    WebviewWindowBuilder::new(app, &label, url)
        .title(format!("Community - {community_name}"))
        .inner_size(900.0, 650.0)
        .min_inner_size(750.0, 500.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .build()
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}

/// W12-fix.C — surface a window for an incoming call.
///
/// Brings the BuddyList window forward (show + focus) and flashes the
/// taskbar / dock icon so the user notices even if they're using another
/// app. Mirrors how Discord/Signal/Telegram make their app
/// unmistakably present when a call arrives.
///
/// On macOS the dock icon bounces; on Windows the taskbar entry
/// flashes; on Linux the window manager sets an urgency hint.
/// Best-effort: failures are logged but never abort the ring path.
pub fn surface_window_for_call(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("buddy-list") {
        if let Err(e) = window.show() {
            tracing::warn!(error = %e, "surface_window_for_call: show failed");
        }
        if let Err(e) = window.unminimize() {
            tracing::trace!(error = %e, "surface_window_for_call: unminimize failed");
        }
        if let Err(e) = window.set_focus() {
            tracing::warn!(error = %e, "surface_window_for_call: set_focus failed");
        }
        // Critical attention type: dock bounce continues until focus,
        // taskbar flash continues until clicked. Acceptable for a
        // call ring — that's the user-attention semantics we want.
        if let Err(e) = window.request_user_attention(Some(tauri::UserAttentionType::Critical)) {
            tracing::trace!(error = %e, "surface_window_for_call: user_attention failed");
        }
    } else {
        tracing::debug!("surface_window_for_call: buddy-list window not found");
    }
}

/// Wave 12 W12.7 — pop out the active call into its own Tauri webview
/// window. The new window mounts the same `<CallController />` (per
/// main.tsx) and a `/call` route that renders the active-call surface.
/// Both webviews see the same call lifecycle events because the
/// backend emits to ALL webviews — they just both reactively render
/// from `callsState`.
pub fn open_call_window(app: &AppHandle, call_id: &str) -> Result<(), String> {
    let label = format!("call-{}", &call_id[..12.min(call_id.len())]);

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let url = WebviewUrl::App(format!("/call?id={call_id}").into());
    WebviewWindowBuilder::new(app, &label, url)
        .title("Call")
        .inner_size(420.0, 540.0)
        .min_inner_size(360.0, 420.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .always_on_top(false)
        .build()
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}

/// Open a profile window for viewing a friend's profile.
pub fn open_profile_window(
    app: &AppHandle,
    public_key: &str,
    display_name: &str,
) -> Result<(), String> {
    let label = format!("profile-{}", &public_key[..12.min(public_key.len())]);

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let url = WebviewUrl::App(format!("/profile?key={public_key}").into());
    WebviewWindowBuilder::new(app, &label, url)
        .title(format!("Profile - {display_name}"))
        .inner_size(380.0, 500.0)
        .min_inner_size(340.0, 420.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .build()
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}
