use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Manager,
};

use crate::services;
use crate::state::{SharedState, UserStatus};
use crate::windows;

pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let online = MenuItem::with_id(app, "status-online", "Online", true, None::<&str>)?;
    let away = MenuItem::with_id(app, "status-away", "Away", true, None::<&str>)?;
    let busy = MenuItem::with_id(app, "status-busy", "Busy", true, None::<&str>)?;
    let offline = MenuItem::with_id(app, "status-offline", "Offline", true, None::<&str>)?;
    let status_menu =
        Submenu::with_items(app, "Status", true, &[&online, &away, &busy, &offline])?;

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
            "status-online" => set_status_from_tray(app, UserStatus::Online),
            "status-away" => set_status_from_tray(app, UserStatus::Away),
            "status-busy" => set_status_from_tray(app, UserStatus::Busy),
            "status-offline" => set_status_from_tray(app, UserStatus::Offline),
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

/// Update the user's status directly from the tray menu and publish to DHT.
fn set_status_from_tray(app: &tauri::AppHandle, status: UserStatus) {
    let state: tauri::State<'_, SharedState> = app.state();

    // Update in-memory state (brief lock, no .await while held)
    {
        let mut identity = state.identity.write();
        if let Some(ref mut id) = *identity {
            id.status = status;
        }
    }

    tracing::info!(status = ?status, "status changed from tray");

    // Clone the Arc<AppState> so we can move it into the async task
    let state_clone = state.inner().clone();

    // Spawn an async task to publish the status change to DHT
    tauri::async_runtime::spawn(async move {
        if let Err(e) = services::presence_service::publish_status(&state_clone, status).await {
            tracing::warn!(error = %e, "failed to publish tray status change to DHT");
        }
    });
}
