//! The skin windows' shell facts the Host's signals follow (plan SK2-A4
//! step 3, design §2.5): each window's OS focus (`OnActivate` /
//! `OnDeactivate`), its opening with what it shows from the start (a chat
//! window's own peer, lead decision 48) and its closing. Only the windows
//! drawn from a skin tree carry signals: the buddy list, chat windows and
//! the pack popups they launch (plan SK2-A5).

use rekindle_types::dto::skin_run::SkinSelection;
use tauri::{AppHandle, Manager, Runtime, Window, WindowEvent};

use crate::state::SharedState;
use crate::window_labels::WindowKind;

/// Whether the window labelled `label` is drawn from a skin tree.
fn is_skin_window(label: &str) -> bool {
    matches!(
        WindowKind::of_label(label),
        Some(WindowKind::BuddyList | WindowKind::Chat | WindowKind::SkinPopup)
    )
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
