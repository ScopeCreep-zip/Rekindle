//! Pack popups in their own windows (plan SK2-A5): Xfire's Infoview
//! (`MainInfoViewPopup`) beside the buddy list, StealthX's main menu. The
//! Host has already decided the popup opens (it answered the pack's
//! `LaunchPopup`); the shell creates or toggles the native window, places it
//! from the served [`ShellPopup`] and keeps it beside its launcher.

use rekindle_types::dto::skin_layout::SkinWindow;
use rekindle_types::dto::skin_tree::ShellPopup;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow, Window, WindowEvent, Wry,
};

use super::popup_place::{popup_frame, WindowBox};
use super::{base_builder, finish, shape};
use crate::state::SharedState;
use crate::window_labels::{self as labels, PopupName, WindowKind};

/// The skin outline whose popups `launcher` opens: the buddy list's or a
/// chat window's (the windows drawn from a skin tree).
///
/// # Errors
/// `launcher` is not a skin window.
pub fn launcher_outline(launcher: &WebviewWindow) -> Result<SkinWindow, String> {
    WindowKind::of_label(launcher.label())
        .and_then(shape::frame_of)
        .ok_or_else(|| "this window launches no popups".to_owned())
}

/// Close `launcher`'s popup `name` if it is open (the launcher's info
/// button toggles it, as Xfire's does). Returns whether it was open.
pub fn close_if_open(app: &AppHandle, launcher: &WebviewWindow, name: &PopupName) -> bool {
    let Some(popup) = app.get_webview_window(&labels::popup_label(launcher.label(), name)) else {
        return false;
    };
    if let Err(e) = popup.destroy() {
        tracing::warn!(error = %e, "could not close a skin popup");
    }
    true
}

/// `window`'s box in whole logical px: its outer position and inner size
/// (frameless, so the two boxes coincide).
fn frame_of(window: &Window<Wry>) -> Result<WindowBox, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let at = window
        .outer_position()
        .map_err(|e| e.to_string())?
        .to_logical::<i32>(scale);
    let size = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<i32>(scale);
    Ok(WindowBox {
        x: at.x,
        y: at.y,
        width: size.width,
        height: size.height,
    })
}

/// Open `launcher`'s popup `name` at `popup`'s place beside it, without
/// taking focus from the launcher: built unfocused and parented to the
/// launcher (macOS child window, Windows owned window, GTK transient), and
/// the launcher's focus handed back after the first show, which keys the
/// new window on macOS (tao `set_visible`).
///
/// # Errors
/// The launcher's frame is unreadable or the window could not be built.
pub fn open(
    app: &AppHandle,
    state: &SharedState,
    launcher: &WebviewWindow,
    name: &PopupName,
    popup: ShellPopup,
) -> Result<(), String> {
    let label = labels::popup_label(launcher.label(), name);
    let frame = popup_frame(&popup, frame_of(&launcher.as_ref().window())?);
    let builder = base_builder(
        app,
        WindowKind::SkinPopup,
        &label,
        format!("/popup/{}", name.as_str()),
    )?
    .title("Rekindle")
    .position(f64::from(frame.x), f64::from(frame.y))
    .inner_size(f64::from(frame.width), f64::from(frame.height))
    .min_inner_size(f64::from(popup.min_width), f64::from(popup.min_height))
    .focused(false)
    .parent(launcher)
    .map_err(|e| e.to_string())?;
    let launcher_focused = matches!(launcher.is_focused(), Ok(true));
    state.skin_popups.lock().insert(label.clone(), popup);
    if let Err(e) = finish(builder, WindowKind::SkinPopup) {
        state.skin_popups.lock().remove(&label);
        return Err(e);
    }
    if launcher_focused {
        if let Err(e) = launcher.set_focus() {
            tracing::debug!(error = %e, "could not hand focus back to a popup's launcher");
        }
    }
    crate::skin_windows::opened(app, &label, None);
    Ok(())
}

/// The window event hook: a launcher that moves or resizes takes its
/// popups along; a launcher that closes closes them; a popup that closes
/// leaves the registry.
pub fn on_window_event(window: &Window<Wry>, event: &WindowEvent) {
    let follows = matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_));
    let destroyed = matches!(event, WindowEvent::Destroyed);
    if !follows && !destroyed {
        return;
    }
    let Some(state) = window.app_handle().try_state::<SharedState>() else {
        return;
    };
    let label = window.label();
    if destroyed {
        state.skin_popups.lock().remove(label);
    }
    let launched: Vec<(String, ShellPopup)> = state
        .skin_popups
        .lock()
        .iter()
        .filter(|(popup, _)| labels::popup_opener(popup) == Some(label))
        .map(|(popup, place)| (popup.clone(), place.clone()))
        .collect();
    if launched.is_empty() {
        return;
    }
    if destroyed {
        for (popup, _) in launched {
            if let Some(popup) = window.app_handle().get_webview_window(&popup) {
                if let Err(e) = popup.destroy() {
                    tracing::warn!(error = %e, "could not close a closed launcher's popup");
                }
            }
        }
        return;
    }
    let launcher = match frame_of(window) {
        Ok(frame) => frame,
        Err(e) => {
            tracing::debug!(error = %e, "a popup's launcher frame is unreadable");
            return;
        }
    };
    for (popup, place) in launched {
        let Some(popup) = window.app_handle().get_webview_window(&popup) else {
            continue;
        };
        let frame = popup_frame(&place, launcher);
        let moved = popup
            .set_position(LogicalPosition::new(frame.x, frame.y))
            .and_then(|()| popup.set_size(LogicalSize::new(frame.width, frame.height)));
        if let Err(e) = moved {
            tracing::debug!(error = %e, "could not move a popup with its launcher");
        }
    }
}
