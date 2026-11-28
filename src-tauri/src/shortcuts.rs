//! Global keyboard shortcut registration and handlers.

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent};
use std::sync::Arc;

use rekindle_lifecycle::SessionScope;

use tauri::Manager;
use tauri_plugin_global_shortcut::{
    Code, Error as ShortcutError, GlobalShortcut, GlobalShortcutExt, Modifiers, Shortcut,
    ShortcutState,
};

use futures::StreamExt;
use rekindle_node::host::panic_shortcut::{self, DesktopSession, PanicRoute};

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
    // Managed first: the panic shortcut's portal route (a Wayland session)
    // needs no OS hot-key manager.
    app.manage(PanicChord::default());
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

/// The panic shortcut registered for the unlocked identity (plan E7.1,
/// quick wipe) and the unlock's scope its trigger runs on, kept so the
/// lock releases exactly it. Managed by [`register`]. An async lock: a
/// portal binding is made and released across awaits, and an arm must not
/// interleave with the unlock's registration.
#[derive(Default)]
pub struct PanicChord(tokio::sync::Mutex<PanicHeld>);

#[derive(Default)]
struct PanicHeld {
    registered: Option<Registered>,
    scope: Option<Arc<SessionScope>>,
}

/// How the held panic shortcut was registered
/// (`rekindle_node::host::panic_shortcut::route`).
enum Registered {
    /// The OS grab, through the global-shortcut plugin.
    Grab(Shortcut),
    /// The GlobalShortcuts portal (a Wayland session).
    Portal(crate::portal_shortcuts::PortalBinding),
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

/// This process's desktop session (`panic_shortcut::DesktopSession`).
fn desktop_session() -> DesktopSession {
    if cfg!(target_os = "linux") {
        DesktopSession::from_linux_env(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        )
    } else {
        DesktopSession::Native
    }
}

/// How a panic shortcut would be delivered on this desktop, decided by
/// the backend's policy (`panic_shortcut::route`); the portal is asked
/// only on a Wayland session.
pub async fn panic_route() -> PanicRoute {
    let session = desktop_session();
    let portal = session == DesktopSession::Wayland && crate::portal_shortcuts::offered().await;
    panic_shortcut::route(session, portal)
}

/// The unlock began: its scope runs the panic shortcut's trigger and the
/// listener that follows the Host's `QuickWipeChanged` (see
/// [`follow_panic_changes`]).
pub async fn begin_panic_session(app: &tauri::AppHandle, scope: &Arc<SessionScope>) {
    if let Some(held) = app.try_state::<PanicChord>() {
        held.0.lock().await.scope = Some(Arc::clone(scope));
    }
}

/// Make `chord` the panic shortcut ([`set_panic_chord`]) and tell the user
/// what they must know: a chord that cannot be delivered (they armed it and
/// must not believe it works), or the other trigger the desktop bound (the
/// GlobalShortcuts portal's dialog). The one registration the unlock and a
/// committed arm or disarm (`QuickWipeChanged`) take.
pub async fn apply_panic_chord(app: &tauri::AppHandle, state: &SharedState, chord: Option<&str>) {
    let alert = match set_panic_chord(app, state, chord).await {
        Ok(Some(bound)) if Some(bound.as_str()) != chord => (
            "Panic shortcut bound",
            format!("The desktop bound the panic shortcut to {bound}."),
        ),
        Ok(_) => return,
        Err(reason) => {
            tracing::error!(%reason, "the panic shortcut is not registered");
            (
                "Panic shortcut not active",
                format!(
                    "{reason}. The duress passphrase still works; choose another shortcut in \
                     Settings → Privacy."
                ),
            )
        }
    };
    // Standing: a security alert stays for the session, so a window that
    // subscribes later still shows it (`SessionJournal::publish_standing`).
    state.host.get().await.publish_standing(
        rekindle_types::subscription_events::SubscriptionEvent::Notification(
            rekindle_types::subscription_events::NotificationEvent::SystemAlert {
                title: alert.0.into(),
                subject: None,
                body: alert.1,
            },
        ),
    );
}

/// How a followed stream of the Host's frames ended.
#[derive(Debug, PartialEq, Eq)]
pub enum FollowEnd {
    /// `InvalidSession` (this listener fell behind) or the stream closed:
    /// take it again, live.
    Resubscribe,
    /// The Host is stopping.
    Stopped,
}

/// Follow the Host's `frames`, handing each `QuickWipeChanged`'s chord to
/// `apply` ([`apply_panic_chord`] in the app) as it comes: the chord of an
/// arming with a shortcut, `None` for a disarm or an arming without one.
pub async fn follow_panic_changes<S, F, Fut>(frames: S, mut apply: F) -> FollowEnd
where
    S: futures::Stream<Item = rekindle_types::subscription_events::StreamFrame>,
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    use rekindle_types::subscription_events::{StreamFrame, SystemEvent};
    futures::pin_mut!(frames);
    while let Some(frame) = frames.next().await {
        match frame {
            StreamFrame::Event(sequenced) => {
                if let SubscriptionEvent::System(SystemEvent::QuickWipeChanged {
                    armed,
                    shortcut,
                }) = sequenced.event
                {
                    apply(shortcut.filter(|_| armed)).await;
                }
            }
            StreamFrame::Resumed(_) => {}
            StreamFrame::InvalidSession => return FollowEnd::Resubscribe,
            StreamFrame::HostStopping { .. } => return FollowEnd::Stopped,
        }
    }
    FollowEnd::Resubscribe
}

/// The unlock ended: the panic shortcut is released and its scope
/// forgotten.
pub async fn end_panic_session(app: &tauri::AppHandle, state: &SharedState) {
    if let Err(e) = set_panic_chord(app, state, None).await {
        tracing::warn!(error = %e, "the panic shortcut was not unregistered");
    }
    if let Some(held) = app.try_state::<PanicChord>() {
        held.0.lock().await.scope = None;
    }
}

/// Make `chord` the panic shortcut (plan E7.1): the previous one is
/// released, and `chord`, when there is one, registered by this desktop's
/// [`panic_route`] to send `QuickWipeTrigger` to the Host on the unlock's
/// scope. `None` (the lock, or a disarm) leaves none. Returns the binding
/// the user is shown: the chord for the OS grab, the compositor's trigger
/// description for the portal. A chord that cannot be delivered is an
/// error: a user who armed it must not believe it works.
///
/// # Errors
/// The chord does not parse, no identity is unlocked, the OS or the
/// portal refused it, or this desktop has no delivery for it.
async fn set_panic_chord(
    app: &tauri::AppHandle,
    state: &SharedState,
    chord: Option<&str>,
) -> Result<Option<String>, String> {
    let next = parse_panic_chord(chord)?;
    let Some(held) = app.try_state::<PanicChord>() else {
        return match next {
            Some(_) => Err("global shortcuts are unavailable on this system".into()),
            None => Ok(None),
        };
    };
    let mut held = held.0.lock().await;
    match held.registered.take() {
        Some(Registered::Grab(previous)) => {
            if let Some(shortcuts) = app.try_state::<GlobalShortcut<tauri::Wry>>() {
                if let Err(e) = shortcuts.unregister(previous) {
                    tracing::warn!(error = %e, "the previous panic shortcut stays registered");
                }
            }
        }
        Some(Registered::Portal(binding)) => binding.close().await,
        None => {}
    }
    let (Some(shortcut), Some(chord)) = (next, chord) else {
        return Ok(None);
    };
    let Some(scope) = held.scope.clone() else {
        return Err("no identity is unlocked".into());
    };
    let (registered, shown) = match panic_route().await {
        PanicRoute::OsGrab => {
            grab(app, state, &scope, shortcut)?;
            (Registered::Grab(shortcut), chord.to_string())
        }
        PanicRoute::Portal => {
            let binding = portal_bind(state, &scope, chord).await?;
            let shown = binding.trigger.clone();
            (Registered::Portal(binding), shown)
        }
        PanicRoute::Unavailable(reason) => return Err(reason.into()),
    };
    held.registered = Some(registered);
    Ok(Some(shown))
}

/// Register `shortcut` with the OS grab.
fn grab(
    app: &tauri::AppHandle,
    state: &SharedState,
    scope: &Arc<SessionScope>,
    shortcut: Shortcut,
) -> Result<(), String> {
    let shortcuts = app
        .try_state::<GlobalShortcut<tauri::Wry>>()
        .ok_or("global shortcuts are unavailable on this system")?;
    let (scope, trigger_state) = (Arc::clone(scope), Arc::clone(state));
    shortcuts
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                trigger_quick_wipe(&scope, Arc::clone(&trigger_state));
            }
        })
        .map_err(|e| format!("the panic shortcut could not be registered: {e}"))
}

/// Bind `chord` through the GlobalShortcuts portal and listen for its
/// presses on `scope` (the lock cancels the listener).
async fn portal_bind(
    state: &SharedState,
    scope: &Arc<SessionScope>,
    chord: &str,
) -> Result<crate::portal_shortcuts::PortalBinding, String> {
    let binding = crate::portal_shortcuts::bind(chord).await?;
    let activations = match binding.activations().await {
        Ok(activations) => activations,
        Err(reason) => {
            binding.close().await;
            return Err(reason);
        }
    };
    let (trigger_scope, trigger_state) = (Arc::clone(scope), Arc::clone(state));
    let listening = scope.spawn("panic shortcut portal", async move {
        futures::pin_mut!(activations);
        while activations.next().await.is_some() {
            trigger_quick_wipe(&trigger_scope, Arc::clone(&trigger_state));
        }
    });
    if listening.is_err() {
        binding.close().await;
        return Err("no identity is unlocked".into());
    }
    Ok(binding)
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
        if crate::windows::skin_load::is_revealed(&window) {
            let _ = window.hide();
            tracing::debug!("buddy list hidden via global shortcut");
        } else {
            crate::windows::reveal_window(&window);
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
        crate::event_dispatch::emit_self_muted(state, new_muted);
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
#[path = "shortcuts_tests.rs"]
mod tests;
