//! Pack popups in their own windows (plan SK2-A5): Xfire's Infoview
//! (`MainInfoViewPopup`) beside the buddy list, StealthX's main menu. The
//! Host has already decided the popup opens (it answered the pack's
//! `LaunchPopup`); the shell creates or toggles the native window, places it
//! from the served [`ShellPopup`] and keeps it beside its launcher.

use rekindle_types::dto::skin_layout::SkinWindow;
use rekindle_types::dto::skin_tree::{ShellPopup, SkinShell};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow, Window, WindowEvent, Wry,
};

use super::popup_place::{follow_frame, popup_frame, served_popup, OpenPopup, WindowBox};
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

/// Move and size `popup` to `frame`, with `min` as its OS minimum (the
/// clamp a user's resize stops at).
fn place(popup: &WebviewWindow, frame: WindowBox, min: Option<(u16, u16)>) {
    let mut placed = popup
        .set_position(LogicalPosition::new(frame.x, frame.y))
        .and_then(|()| popup.set_size(LogicalSize::new(frame.width, frame.height)));
    if let Some((w, h)) = min {
        placed = placed
            .and_then(|()| popup.set_min_size(Some(LogicalSize::new(f64::from(w), f64::from(h)))));
    }
    if let Err(e) = placed {
        tracing::debug!(error = %e, "could not place a popup beside its launcher");
    }
}

/// Open `launcher`'s popup `name` at `popup`'s place beside it, without
/// taking focus from the launcher: built unfocused and parented to the
/// launcher (macOS child window, Windows owned window, GTK transient), and
/// the launcher's focus handed back after the first show, which keys the
/// new window on macOS (tao `set_visible`). Resizable (the base builder),
/// down to the popup's minimum (`min_inner_size`, the OS clamp of the
/// pack's resize tiles).
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
    let open = OpenPopup {
        place: popup,
        laid: (frame.width, frame.height),
    };
    state.skin_popups.lock().insert(label.clone(), open);
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

/// A popup window's current inner size in whole logical px.
fn current_size(popup: &WebviewWindow) -> Result<(i32, i32), String> {
    let scale = popup.scale_factor().map_err(|e| e.to_string())?;
    let size = popup
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<i32>(scale);
    Ok((size.width, size.height))
}

/// The window event hook: a launcher that moves or resizes takes its
/// popups along (a popup the user resized keeps its size); a launcher that
/// closes closes them; a popup that closes leaves the registry.
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
    let launched: Vec<(String, OpenPopup)> = state
        .skin_popups
        .lock()
        .iter()
        .filter(|(popup, _)| labels::popup_opener(popup) == Some(label))
        .map(|(popup, open)| (popup.clone(), open.clone()))
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
    for (label, open) in launched {
        let Some(popup) = window.app_handle().get_webview_window(&label) else {
            continue;
        };
        let current = match current_size(&popup) {
            Ok(size) => size,
            Err(e) => {
                tracing::debug!(error = %e, "a popup's size is unreadable");
                continue;
            }
        };
        let frame = follow_frame(&open, launcher, current);
        if current == open.laid {
            if let Some(entry) = state.skin_popups.lock().get_mut(&label) {
                entry.laid = (frame.width, frame.height);
            }
        }
        place(&popup, frame, None);
    }
}

/// The shown skin changed (`SystemEvent::SkinChanged`, its served
/// `shells`): each open popup is laid afresh from the new skin's popup of
/// its name for its launcher's outline (a new skin is a fresh layout, size
/// included), or closed when the new skin has none.
pub fn skin_changed(app: &AppHandle, state: &SharedState, shells: &[SkinShell]) {
    let open: Vec<String> = state.skin_popups.lock().keys().cloned().collect();
    for label in open {
        let Some(popup) = app.get_webview_window(&label) else {
            continue;
        };
        let served = labels::popup_parts(&label).and_then(|(opener, name)| {
            let outline = WindowKind::of_label(opener).and_then(shape::frame_of)?;
            let launcher = app.get_webview_window(opener)?;
            Some((served_popup(shells, outline, name), launcher))
        });
        let Some((Some(place_now), launcher)) = served else {
            if let Err(e) = popup.destroy() {
                tracing::warn!(error = %e, "could not close a popup the new skin lacks");
            }
            continue;
        };
        let frame = match frame_of(&launcher.as_ref().window()) {
            Ok(opener) => popup_frame(&place_now, opener),
            Err(e) => {
                tracing::debug!(error = %e, "a popup's launcher frame is unreadable");
                continue;
            }
        };
        let min = (place_now.min_width, place_now.min_height);
        state.skin_popups.lock().insert(
            label,
            OpenPopup {
                place: place_now,
                laid: (frame.width, frame.height),
            },
        );
        place(&popup, frame, Some(min));
    }
}
