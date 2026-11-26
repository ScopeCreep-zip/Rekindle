//! The skin windows' shell facts the Host's signals follow (plan SK2-A4
//! step 3, design §2.5): each window's OS focus (`OnActivate` /
//! `OnDeactivate`), its opening with what it shows from the start (a chat
//! window's own peer, lead decision 48) and its closing. Only the windows
//! drawn from a skin tree carry signals: the buddy list, chat windows, a
//! community window (plan SKW1: the pack's chat frame) and the pack popups
//! they launch (plan SK2-A5). The community browser is no skin tree.
//!
//! The shell announces its windows; the Host never enumerates them (ADR
//! 0010). A window opened while a Host runs announces itself (`opened`);
//! the ones already open when the Host starts are announced then
//! (`announce_open`), with the focused one marked, so their signals never
//! wait for an OS focus event (plan CR1: the buddy list was a signal
//! window only if one reached it after login).

use rekindle_types::dto::skin_run::SkinSelection;
use tauri::{AppHandle, Manager, Runtime, Window, WindowEvent};

use crate::state::SharedState;
use crate::window_labels::WindowKind;

/// Whether the window labelled `label` is drawn from a skin tree: the
/// windows wearing a pack frame (`frame_of_label`, which leaves out the
/// community browser) and the pack popups.
fn is_skin_window(label: &str) -> bool {
    crate::windows::frame_of_label(label).is_some()
        || matches!(WindowKind::of_label(label), Some(WindowKind::SkinPopup))
}

fn host<R: Runtime>(app: &AppHandle<R>) -> Option<std::sync::Arc<rekindle_node::host::Host>> {
    app.try_state::<SharedState>()?.host.current()
}

/// The Tauri window event hook: focus and close of a skin window.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let label = window.label();
    if !is_skin_window(label) {
        return;
    }
    let Some(host) = host(window.app_handle()) else {
        return;
    };
    match event {
        WindowEvent::Focused(focused) => host.skin_window_focused(label, *focused),
        WindowEvent::Destroyed => host.skin_window_closed(label),
        _ => {}
    }
}

/// The Host just started: announce the skin windows already open, and
/// which of them has focus.
pub fn announce_open<R: Runtime>(app: &AppHandle<R>, host: &rekindle_node::host::Host) {
    let open: Vec<(String, bool)> = app
        .webview_windows()
        .into_iter()
        .map(|(label, window)| {
            let focused = matches!(window.is_focused(), Ok(true));
            (label, focused)
        })
        .collect();
    announce(host, &open);
}

/// Announce the open windows `open` (label, focused) to `host`. Only the
/// buddy list can be open before the Host: a chat window or a pack popup
/// is opened by a session command once the Host is set, and announces
/// itself with its selection and launcher (`opened`), which an
/// announcement here must not overwrite. The focused skin window is then
/// marked focused, as its next OS focus event would.
fn announce(host: &rekindle_node::host::Host, open: &[(String, bool)]) {
    for (label, _) in open {
        if matches!(WindowKind::of_label(label), Some(WindowKind::BuddyList)) {
            host.skin_window_opened(label, None, None);
        }
    }
    for (label, focused) in open {
        if *focused && is_skin_window(label) {
            host.skin_window_focused(label, true);
        }
    }
}

/// A skin window opened showing `selection` (a chat window: its peer);
/// `launcher` is the window that launched it when it is a pack popup.
pub fn opened<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    launcher: Option<&str>,
    selection: Option<SkinSelection>,
) {
    if let Some(host) = host(app) {
        host.skin_window_opened(label, launcher, selection);
    }
}

#[cfg(test)]
#[path = "skin_windows_tests.rs"]
mod tests;
