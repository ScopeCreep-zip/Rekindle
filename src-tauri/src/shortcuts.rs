//! Global keyboard shortcut registration and handlers.

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent};
use std::sync::Arc;

use rekindle_lifecycle::SessionScope;

use tauri::Manager;
use tauri_plugin_global_shortcut::{
    Code, Error as ShortcutError, GlobalShortcut, GlobalShortcutExt, Modifiers, Shortcut,
    ShortcutState,
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
    app.manage(PanicChord::default());
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

/// The panic shortcut registered for the unlocked identity (plan E7.1,
/// quick wipe) and the unlock's scope its trigger runs on, kept so the
/// lock unregisters exactly it. Managed by [`register`].
#[derive(Default)]
pub struct PanicChord(parking_lot::Mutex<PanicHeld>);

#[derive(Default)]
struct PanicHeld {
    shortcut: Option<Shortcut>,
    scope: Option<Arc<SessionScope>>,
}

/// The stored `app_settings.panic_shortcut` as a chord. The Host checked
/// the accelerator when it was armed; one the plugin cannot parse is an
/// error, never a silently unarmed shortcut.
///
/// # Errors
/// The plugin's parser refuses the accelerator.
pub fn parse_panic_chord(stored: Option<&str>) -> Result<Option<Shortcut>, String> {
    stored
        .map(|chord| {
            chord
                .parse::<Shortcut>()
                .map_err(|e| format!("the panic shortcut `{chord}` is not a chord: {e}"))
        })
        .transpose()
}

/// The unlock began: its scope runs the panic shortcut's trigger, and the
/// stored `chord` is registered (see [`set_panic_chord`]).
///
/// # Errors
/// As [`set_panic_chord`].
pub fn begin_panic_session(
    app: &tauri::AppHandle,
    state: &SharedState,
    scope: &Arc<SessionScope>,
    chord: Option<&str>,
) -> Result<(), String> {
    if let Some(held) = app.try_state::<PanicChord>() {
        held.0.lock().scope = Some(Arc::clone(scope));
    }
    set_panic_chord(app, state, chord)
}

/// The unlock ended: the panic shortcut is unregistered and its scope
/// forgotten.
pub fn end_panic_session(app: &tauri::AppHandle, state: &SharedState) {
    if let Err(e) = set_panic_chord(app, state, None) {
        tracing::warn!(error = %e, "the panic shortcut was not unregistered");
    }
    if let Some(held) = app.try_state::<PanicChord>() {
        held.0.lock().scope = None;
    }
}

/// Make `chord` the panic shortcut (plan E7.1): the previous one is
/// unregistered, and `chord`, when there is one, registered to send
/// `QuickWipeTrigger` to the Host on the unlock's scope. `None` (the lock,
/// or a disarm) leaves none. A chord the OS refuses is an error: a user
/// who armed it must not believe it works.
///
/// # Errors
/// The chord does not parse, no identity is unlocked, the OS refused it,
/// or global shortcuts are unavailable on this system.
pub fn set_panic_chord(
    app: &tauri::AppHandle,
    state: &SharedState,
    chord: Option<&str>,
) -> Result<(), String> {
    let next = parse_panic_chord(chord)?;
    let (Some(held), Some(shortcuts)) = (
        app.try_state::<PanicChord>(),
        app.try_state::<GlobalShortcut<tauri::Wry>>(),
    ) else {
        return match next {
            Some(_) => Err("global shortcuts are unavailable on this system".into()),
            None => Ok(()),
        };
    };
    let mut held = held.0.lock();
    if let Some(previous) = held.shortcut.take() {
        if let Err(e) = shortcuts.unregister(previous) {
            tracing::warn!(error = %e, "the previous panic shortcut stays registered");
        }
    }
    let Some(shortcut) = next else {
        return Ok(());
    };
    let Some(scope) = held.scope.clone() else {
        return Err("no identity is unlocked".into());
    };
    let trigger_state = Arc::clone(state);
    shortcuts
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                trigger_quick_wipe(&scope, Arc::clone(&trigger_state));
            }
        })
        .map_err(|e| format!("the panic shortcut could not be registered: {e}"))?;
    held.shortcut = Some(shortcut);
    Ok(())
}

/// The panic shortcut was pressed: the Host wipes (`QuickWipeTrigger`,
/// Routine, no review; plan E7.1). The Host refuses it with 409 while no
/// shortcut is armed, and stops itself after the wipe; the GUI follows its
/// `HostStopping { DataWiped }`.
fn trigger_quick_wipe(scope: &Arc<SessionScope>, state: SharedState) {
    scope.spawn_or_drop("panic shortcut", async move {
        let sent = crate::host_call::call::<serde_json::Value>(
            &state,
            rekindle_ipc::protocol::IpcRequest::QuickWipeTrigger,
        )
        .await;
        if let Err(e) = sent {
            tracing::error!(error = %e, "the quick wipe was refused");
        }
    });
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

    /// A stored panic shortcut becomes the chord the plugin registers; a
    /// malformed one is an error, never a quietly unarmed shortcut.
    #[test]
    fn the_panic_chord_parses_or_is_refused() {
        let chord = parse_panic_chord(Some("CommandOrControl+Shift+F12"))
            .unwrap()
            .unwrap();
        assert_eq!(
            chord,
            "CommandOrControl+Shift+F12".parse::<Shortcut>().unwrap()
        );
        assert!(parse_panic_chord(None).unwrap().is_none());
        assert!(parse_panic_chord(Some("Shift+NotAKey")).is_err());
    }

    #[test]
    fn main_thread_dispatch_failure_stays_an_error() {
        let failed = Err(ShortcutError::RecvError(std::sync::mpsc::RecvError));
        assert!(settle_registration("ctrl+shift+m", failed).is_err());
    }
}
