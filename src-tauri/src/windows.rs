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
    let id_part = if community_id.is_empty() { "browser" } else { &community_id[..16.min(community_id.len())] };
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
