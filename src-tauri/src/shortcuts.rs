//! Global keyboard shortcut registration and handlers.

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent};
use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_global_shortcut::{
    Code, Error as ShortcutError, GlobalShortcutExt, Modifiers, ShortcutState,
};

use crate::state::SharedState;

/// The fixed chords: toggle the buddy list, toggle voice mute. macOS uses
/// Cmd (Super); Windows/Linux use Ctrl.
#[cfg(target_os = "macos")]
const CHORDS: [&str; 2] = ["super+shift+x", "super+shift+m"];
#[cfg(not(target_os = "macos"))]
const CHORDS: [&str; 2] = ["ctrl+shift+x", "ctrl+shift+m"];

/// Register the global keyboard shortcuts plugin with state-aware handlers.
///
/// The plugin is built with the handler only and each chord is registered
/// afterwards on its own, so a chord the OS refuses (another program, or a
/// second Rekindle instance, already holds it) leaves the app running
/// without that chord instead of failing plugin setup. The plugin is
/// registered here (rather than in the builder chain) so the handler can
/// capture [`SharedState`].
pub fn register(app: &tauri::App, state: &SharedState) -> Result<(), Box<dyn std::error::Error>> {
    let shortcut_state = Arc::clone(state);
    let plugin = tauri_plugin_global_shortcut::Builder::new()
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
        .build();
    match app.handle().plugin(plugin) {
        Ok(()) => {}
        // The OS hot-key manager could not start (`GlobalHotKeyManager::new`
        // in the plugin's setup): the app runs with no global shortcuts.
        Err(tauri::Error::PluginInitialization(_, reason)) => {
            tracing::warn!(%reason, "global shortcuts unavailable");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    }
    let shortcuts = app.global_shortcut();
    for chord in CHORDS {
        settle_registration(chord, shortcuts.register(chord))?;
    }
    Ok(())
}

/// Settle one chord's registration result. The OS refusing the chord
/// (`Error::GlobalHotkey`: already registered by another program or another
/// instance, or rejected) is logged and the app continues without it; a
/// runtime failure dispatching to the main thread stays an error.
fn settle_registration(
    chord: &str,
    result: Result<(), ShortcutError>,
) -> Result<(), ShortcutError> {
    match result {
        Ok(()) => Ok(()),
        Err(ShortcutError::GlobalHotkey(reason)) => {
            tracing::warn!(chord, %reason, "global shortcut unavailable");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Toggle the buddy list window visibility (Ctrl+Shift+X / Cmd+Shift+X).
fn toggle_buddy_list(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window(crate::window_labels::BUDDY_LIST) {
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
        // Only meaningful inside a call: muting with no session running
        // has no scope to report against, and `toggle_voice_mute`
        // already returns `None` in that case.
        if let Some(scope) = crate::state_helpers::current_voice_scope(state) {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &SubscriptionEvent::Voice(VoiceEvent::MuteChanged {
                    scope,
                    target_pseudonym: public_key,
                    muted: new_muted,
                }),
            );
        }

        tracing::debug!(muted = new_muted, "voice mute toggled via global shortcut");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chord another program or a second instance holds leaves the app
    /// running (Linux finding: `HotKey already registered` panicked setup).
    #[test]
    fn already_registered_chord_is_not_fatal() {
        let refused = Err(ShortcutError::GlobalHotkey(
            "HotKey already registered: HotKey { mods: CONTROL | SHIFT, key: KeyX }".into(),
        ));
        assert!(settle_registration("ctrl+shift+x", refused).is_ok());
        assert!(settle_registration("ctrl+shift+x", Ok(())).is_ok());
    }

    #[test]
    fn main_thread_dispatch_failure_stays_an_error() {
        let failed = Err(ShortcutError::RecvError(std::sync::mpsc::RecvError));
        assert!(settle_registration("ctrl+shift+m", failed).is_err());
    }
}
