use std::sync::Arc;

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

    // "Show Rekindle" first: on hosts whose tray items are menu-only
    // (Ayatana/StatusNotifierItem on COSMIC and others), a click opens this
    // menu and never reaches `on_tray_icon_event`, so the menu must reach the
    // buddy list too (Linux finding T1).
    let show = MenuItem::with_id(app, "show", "Show Rekindle", true, None::<&str>)?;
    let show_sep = PredefinedMenuItem::separator(app)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Rekindle", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &show_sep,
            &status_menu,
            &sep,
            &settings,
            &sep2,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(TrayIcon::LoggedOut.bytes())?)
        .icon_as_template(TrayIcon::LoggedOut.is_template())
        .tooltip("Rekindle")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match TrayAction::of(event.id.as_ref()) {
            Some(TrayAction::Quit) => quit_rekindle(app),
            // The same path as the tray's left-click.
            Some(TrayAction::Show) => show_from_tray(app),
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
                show_from_tray(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// The tray icon's id, by which a status change finds it.
const TRAY_ID: &str = "rekindle";

fn show_from_tray(app: &tauri::AppHandle) {
    if let Some(login) = app.get_webview_window(crate::window_labels::LOGIN) {
        windows::reveal_window(&login);
        return;
    }
    let state = Arc::clone(app.state::<SharedState>().inner());
    let app = app.clone();
    let task_state = Arc::clone(&state);
    let runtime = tauri::async_runtime::handle();
    let _entered = runtime.inner().enter();
    crate::state_helpers::spawn_in_login(&task_state, "open buddy list from tray", async move {
        if let Err(error) = windows::open_buddy_list(&app, &state).await {
            tracing::warn!(%error, "could not open buddy list from tray");
        }
    });
}

/// The icon the tray shows (plan E7.2 "Tray"): the app mark while no one is
/// logged in, else one of four pre-rendered icons carrying the status dot
/// (`icons/tray/`, the status colours of `ui-skin.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayIcon {
    LoggedOut,
    Online,
    Away,
    Busy,
    Offline,
}

impl TrayIcon {
    /// The icon for our status; Invisible shows as offline, as peers see it.
    fn of(status: Option<SessionStatus>) -> Self {
        match status {
            None => Self::LoggedOut,
            Some(SessionStatus::Online) => Self::Online,
            Some(SessionStatus::Away) => Self::Away,
            Some(SessionStatus::Busy) => Self::Busy,
            Some(SessionStatus::Offline | SessionStatus::Invisible) => Self::Offline,
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Self::LoggedOut => include_bytes!("../icons/icon.png"),
            Self::Online => include_bytes!("../icons/tray/online.png"),
            Self::Away => include_bytes!("../icons/tray/away.png"),
            Self::Busy => include_bytes!("../icons/tray/busy.png"),
            Self::Offline => include_bytes!("../icons/tray/offline.png"),
        }
    }

    /// macOS draws a template icon monochrome, which would erase the dot's
    /// colour: only the plain app mark is a template.
    fn is_template(self) -> bool {
        self == Self::LoggedOut
    }
}

/// Show our status in the tray icon: `None` once the session has ended. Called
/// on the Host's `SelfChanged` (`event_dispatch.rs`) and at the session's end.
pub fn show_status(app: &tauri::AppHandle, status: Option<SessionStatus>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let icon = TrayIcon::of(status);
    let image = match Image::from_bytes(icon.bytes()) {
        Ok(image) => image,
        Err(e) => {
            tracing::warn!(error = %e, "tray icon did not decode");
            return;
        }
    };
    if let Err(e) = tray.set_icon(Some(image)) {
        tracing::warn!(error = %e, "tray icon not set");
    }
    if let Err(e) = tray.set_icon_as_template(icon.is_template()) {
        tracing::warn!(error = %e, "tray icon template flag not set");
    }
}

/// What a tray menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    Quit,
    /// Reveal the buddy list (the left-click's action, for menu-only trays).
    Show,
    /// Open Settings, on a tab or where it was.
    OpenSettings(Option<windows::SettingsTab>),
    SetStatus(SessionStatus),
}

impl TrayAction {
    /// The action of the menu item with `id`.
    fn of(id: &str) -> Option<Self> {
        Some(match id {
            "quit" => Self::Quit,
            "show" => Self::Show,
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
#[path = "tray_tests.rs"]
mod tests;
