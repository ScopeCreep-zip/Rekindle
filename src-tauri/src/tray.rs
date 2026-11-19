use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Manager,
};

use rekindle_types::presence::SessionStatus;

use crate::state::SharedState;
use crate::windows;

pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let online = MenuItem::with_id(app, "status-online", "Online", true, None::<&str>)?;
    let away = MenuItem::with_id(app, "status-away", "Away", true, None::<&str>)?;
    let busy = MenuItem::with_id(app, "status-busy", "Busy", true, None::<&str>)?;
    let offline = MenuItem::with_id(app, "status-offline", "Appear Offline", true, None::<&str>)?;
    let status_sep = PredefinedMenuItem::separator(app)?;
    // Plan E7.2: Xfire's tray set a custom status too; it opens the status
    // message editor (Settings → Profile).
    let custom = MenuItem::with_id(
        app,
        "status-message",
        "Set custom status\u{2026}",
        true,
        None::<&str>,
    )?;
    let status_menu = Submenu::with_items(
        app,
        "Status",
        true,
        &[&online, &away, &busy, &offline, &status_sep, &custom],
    )?;

    let sep = PredefinedMenuItem::separator(app)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Rekindle", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&status_menu, &sep, &settings, &sep2, &quit])?;

    TrayIconBuilder::new()
        .icon(Image::from_bytes(include_bytes!("../icons/icon.png"))?)
        .icon_as_template(true)
        .tooltip("Rekindle")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match TrayAction::of(event.id.as_ref()) {
            Some(TrayAction::Quit) => quit_rekindle(app),
            Some(TrayAction::OpenSettings(tab)) => {
                let _ = windows::open_settings(app, tab);
            }
            Some(TrayAction::SetStatus(status)) => set_status_from_tray(app, status),
            None => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                let _ = windows::open_buddy_list(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// What a tray menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    Quit,
    /// Open Settings, on a tab or where it was.
    OpenSettings(Option<windows::SettingsTab>),
    SetStatus(SessionStatus),
}

impl TrayAction {
    /// The action of the menu item with `id`.
    fn of(id: &str) -> Option<Self> {
        Some(match id {
            "quit" => Self::Quit,
            "settings" => Self::OpenSettings(None),
            // "Set custom status…": the status message editor.
            "status-message" => Self::OpenSettings(Some(windows::SettingsTab::Profile)),
            "status-online" => Self::SetStatus(SessionStatus::Online),
            "status-away" => Self::SetStatus(SessionStatus::Away),
            "status-busy" => Self::SetStatus(SessionStatus::Busy),
            // Appear Offline: invisible, which peers cannot tell from
            // offline (only a lock or logout writes offline).
            "status-offline" => Self::SetStatus(SessionStatus::Invisible),
            _ => return None,
        })
    }
}

/// Quit Rekindle: the tray's Quit and the menu bar's (`quit_app`). A
/// programmatic exit, which the run loop lets proceed (`lib.rs`,
/// `ExitRequested` with a code) and which ends in the graceful shutdown
/// (`RunEvent::Exit`, `shutdown::handle_exit`).
pub fn quit_rekindle(app: &tauri::AppHandle) {
    app.exit(0);
}

/// Set our status from the tray: the same Host call as the status picker
/// (`IpcRequest::PresenceSet`).
fn set_status_from_tray(app: &tauri::AppHandle, status: SessionStatus) {
    let state = app.state::<SharedState>().inner().clone();
    // A status is the session's: logged out, there is none to set.
    let task_state = std::sync::Arc::clone(&state);
    crate::state_helpers::spawn_in_login(&task_state, "status from tray", async move {
        let request = rekindle_ipc::protocol::IpcRequest::PresenceSet { status };
        match crate::host_call::call::<serde_json::Value>(&state, request).await {
            Ok(_) => tracing::info!(?status, "status changed from tray"),
            Err(e) => tracing::warn!(error = %e, "status from tray refused"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_custom_status_opens_the_status_message_editor() {
        assert_eq!(
            TrayAction::of("status-message"),
            Some(TrayAction::OpenSettings(Some(
                windows::SettingsTab::Profile
            )))
        );
        assert_eq!(
            TrayAction::of("settings"),
            Some(TrayAction::OpenSettings(None))
        );
        assert_eq!(
            TrayAction::of("status-offline"),
            Some(TrayAction::SetStatus(SessionStatus::Invisible))
        );
        assert_eq!(TrayAction::of("unknown"), None);
    }
}
