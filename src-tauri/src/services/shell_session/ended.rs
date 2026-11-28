//! `ShellHooks::ending` and `ended`: what the GUI does at the end of an
//! unlock. The Host's teardown runs between the two: the scope's tasks, the
//! Offline write, the record pool, our routes and the identity's keys and
//! stores.

use std::sync::Arc;
use std::time::Duration;

use rekindle_lifecycle::SessionScope;
use rekindle_node::host::EndReason;
use tauri::Manager as _;

use crate::services;
use crate::state::{AppState, SharedState};

/// How long the end waits for us to leave calls and voice (plan C7.27): the
/// `CallEnd`s, the `VoiceLeave` and the presence row without a voice claim.
/// Each step runs on to completion past it; only the teardown's wait is
/// bounded.
const LEAVE_ON_END_WAIT: Duration = Duration::from_secs(3);

/// Before the Host cancels the unlock scope, while the session can still
/// send: leave calls and voice (→ E4.1), and flush the peer-reliability
/// counters while the database is open (→ E3.1).
pub(super) async fn ending(state: &SharedState, app: Option<&tauri::AppHandle>) {
    if let Some(app) = app {
        leave_calls(app, state).await;
    }
    if let Ok(pool) = state.db.current() {
        services::community::flush_peer_reliability(state, &pool).await;
    }
}

/// The identity is out of the Host: clear what the GUI held of it, and show
/// the login window when the user is signed out.
pub(super) async fn ended(state: &SharedState, app: Option<&tauri::AppHandle>, reason: EndReason) {
    services::native_video::stop(state);
    let signed_out = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone());
    clear(state);
    let Some(app) = app else {
        return;
    };
    // The panic shortcut is the unlocked identity's (plan E7.1).
    crate::shortcuts::end_panic_session(app, state).await;
    services::veilid::emit_network_status(app, state);
    // No session, no status: the tray shows the app mark again (plan E7.2).
    crate::tray::show_status(app, None);
    match reason {
        EndReason::Logout | EndReason::Deleted => show_login(app, signed_out.as_deref()),
        EndReason::TaskPanicked(task) => {
            crate::event_dispatch::emit_notification(
                app,
                rekindle_types::subscription_events::NotificationEvent::SystemAlert {
                    title: "Session ended".into(),
                    subject: None,
                    body: format!(
                        "An internal error ({task}) ended your session. Log in again to continue."
                    ),
                },
            );
            show_login(app, signed_out.as_deref());
        }
        // A failed unlock leaves the login window up; at exit no window
        // follows; the GUI has no lock but logout.
        EndReason::Failed | EndReason::Exit | EndReason::Lock => {}
    }
}

/// Everything the GUI held for the identity, so none of it reaches the
/// next one to log in.
fn clear(state: &AppState) {
    *state.identity.write() = None;
    *state.identity_secret.lock() = None;
    services::preferences::reset(state);
    // A deep link belongs to the user it reached (the Host's teardown
    // cleared the identity's friend state, plan E2.1).
    *state.held_deep_link.lock() = None;
    state.pending_link_opens.lock().clear();
    if let Some(nh) = state.node.write().as_mut() {
        nh.profile_dht_key = None;
        nh.friend_list_dht_key = None;
        nh.mailbox_dht_key = None;
    }
    *state.personal_sync_lease.lock() = None;
    state.communities.write().clear();
    // Each cache is the identity's own keyed tree (plan E3.0a).
    crate::state_helpers::clear_meks(state);
    state.dedup_cache.lock().clear();
    tracing::info!("session state cleared — node still running");
}

/// The login window, with the identity that signed out preselected, and no
/// other window.
fn show_login(app: &tauri::AppHandle, signed_out: Option<&str>) {
    let preselect = match signed_out
        .map(rekindle_types::key_format::public_key_hex)
        .transpose()
    {
        Ok(preselect) => preselect,
        Err(e) => {
            tracing::warn!(error = %e, "the signed-out identity's key is malformed");
            None
        }
    };
    if let Err(e) = crate::windows::open_login(app, preselect.as_ref()) {
        tracing::error!(error = %e, "the login window did not open");
    }
    for (label, window) in app.webview_windows() {
        if label != crate::window_labels::LOGIN {
            let _ = window.destroy();
        }
    }
}

/// End the desktop's unported call signaling before the Host closes the unlock.
async fn leave_calls(app: &tauri::AppHandle, state: &Arc<AppState>) {
    let Ok(pool) = state.db.current() else { return };
    let scope = SessionScope::new(
        "session end",
        Arc::new(|task| tracing::error!(task, "session end task panicked")),
    );
    let calls = services::calls_adapter::CallsAdapter::new(Arc::clone(state), app.clone(), pool);
    scope.spawn_or_drop("end calls", async move {
        rekindle_calls::signaling::end_all_calls(calls.as_ref(), "signed out").await;
    });
    if let Err(stuck) = scope.shutdown(LEAVE_ON_END_WAIT).await {
        tracing::info!(%stuck, "call signaling ran past the session end deadline");
    }
}
