//! Global keyboard shortcut registration and handlers.

use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};

use crate::channels;
use crate::state::SharedState;

/// Register the global keyboard shortcuts plugin with state-aware handlers.
///
/// macOS uses Cmd (Super); Windows/Linux use Ctrl. The plugin is registered
/// here (rather than in the builder chain) so the handler can capture
/// [`SharedState`].
pub fn register(app: &tauri::App, state: &SharedState) -> Result<(), Box<dyn std::error::Error>> {
    let shortcut_state = Arc::clone(state);
    #[cfg(target_os = "macos")]
    let builder = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcuts(["super+shift+x", "super+shift+m"])?;
    #[cfg(not(target_os = "macos"))]
    let builder = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcuts(["ctrl+shift+x", "ctrl+shift+m"])?;
    app.handle().plugin(
        builder
            .with_handler(move |app_handle, shortcut, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }

                // Ctrl+Shift+X / Cmd+Shift+X — toggle buddy list visibility
                if shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyX)
                    || shortcut.matches(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyX)
                {
                    toggle_buddy_list(app_handle);
                }

                // Ctrl+Shift+M / Cmd+Shift+M — toggle voice mute
                if shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyM)
                    || shortcut.matches(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyM)
                {
                    toggle_mute(app_handle, &shortcut_state);
                }
            })
            .build(),
    )?;
    Ok(())
}

/// Toggle the buddy list window visibility (Ctrl+Shift+X / Cmd+Shift+X).
fn toggle_buddy_list(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("buddy-list") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            tracing::debug!("buddy list hidden via global shortcut");
        } else {
            let _ = window.show();
            let _ = window.set_focus();
            tracing::debug!("buddy list shown via global shortcut");
        }
    }
}

/// Toggle voice mute state (Ctrl+Shift+M / Cmd+Shift+M).
///
/// Flips the mute flag on the voice engine and emits a `VoiceEvent::UserMuted`
/// event so the frontend stays in sync.
fn toggle_mute(app_handle: &tauri::AppHandle, state: &SharedState) {
    // Read identity key first to avoid holding two locks simultaneously
    let public_key = crate::state_helpers::owner_key_or_default(state);

    if let Some(new_muted) = state.toggle_voice_mute() {
        let event = channels::VoiceEvent::UserMuted {
            public_key,
            muted: new_muted,
        };
        crate::event_dispatch::emit_live(app_handle, "voice-event", &event);

        tracing::debug!(muted = new_muted, "voice mute toggled via global shortcut");
    }
}
