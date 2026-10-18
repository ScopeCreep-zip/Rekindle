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
    let status_menu = Submenu::with_items(app, "Status", true, &[&online, &away, &busy, &offline])?;

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
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => {
                let _ = windows::open_settings(app, None);
            }
            "status-online" => set_status_from_tray(app, SessionStatus::Online),
            "status-away" => set_status_from_tray(app, SessionStatus::Away),
            "status-busy" => set_status_from_tray(app, SessionStatus::Busy),
            // Appear Offline: invisible, which peers cannot tell from
            // offline (only a lock or logout writes offline).
            "status-offline" => set_status_from_tray(app, SessionStatus::Invisible),
            _ => {}
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
