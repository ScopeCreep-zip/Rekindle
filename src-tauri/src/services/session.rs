//! The login session: the scope its tasks run in, and the one teardown
//! that ends it (plan C4).
//!
//! Every task that belongs to a login spawns through the login scope, so
//! ending the session ends all of them. `end_session` is the single
//! teardown for logout, deleting the active identity and app exit, in the
//! plan's order: the scope (every session task) first, then the work that
//! needs the session's keys and records (offline status, record close,
//! flushes), then the in-memory state, and the keystore last.

use std::sync::Arc;
use std::time::Duration;

use rekindle_lifecycle::SessionScope;
use tauri::Manager as _;

use crate::commands::voice::{shutdown_voice, VoiceShutdownOpts};
use crate::keystore::KeystoreHandle;
use crate::services;
use crate::state::{AppState, SharedState, UserStatus};
use crate::state_helpers;

/// How long the login scope's tasks get to stop at logout or exit: one
/// in-flight Veilid call plus margin, then they are aborted.
const SCOPE_STOP_DEADLINE: Duration = rekindle_protocol::veilid_config::SESSION_STOP_DEADLINE;

/// How long logout waits for the Offline status write (plan C7.6j). The
/// write is one record-pool call: it runs on to completion past this, and
/// the record closes after it; only the user's wait is bounded. Signal
/// Desktop bounds its shutdown queue drain the same way (`10 * SECOND`).
const OFFLINE_WRITE_WAIT: Duration = Duration::from_secs(10);

/// Start the login session's scope. A task that panics ends the session:
/// the state it shared with the other tasks may be half-mutated
/// (`evidence/c4-session-scope-research.md`), so the user is logged out
/// and told why, and the app stays up.
pub fn begin(app: &tauri::AppHandle, state: &SharedState) -> Arc<SessionScope> {
    let app = app.clone();
    let scope = SessionScope::new(
        "login",
        Arc::new(move |task| {
            let app = app.clone();
            // App-lifetime: the login scope is the thing being ended.
            tauri::async_runtime::spawn(async move {
                crate::event_dispatch::emit_notification(
                    &app,
                    rekindle_types::subscription_events::NotificationEvent::SystemAlert {
                        title: "Session ended".into(),
                        body: format!(
                            "An internal error ({task}) ended your session. Log in again to continue."
                        ),
                    },
                );
                let state = app.state::<SharedState>().inner().clone();
                let keystore = app.state::<KeystoreHandle>().inner().clone();
                if let Err(e) =
                    services::auth_runtime::logout_inner(app.clone(), state, keystore).await
                {
                    tracing::error!(error = %e, "ending the session after a task panic failed");
                }
            });
        }),
    );
    *state.login_scope.write() = Some(Arc::clone(&scope));
    scope
}

/// End the login session. `app` is `None` at app exit, when no window is
/// left to update.
pub async fn end_session(
    app: Option<&tauri::AppHandle>,
    state: &Arc<AppState>,
    keystore: &KeystoreHandle,
) {
    // 1. Every session task. The stop token first, so a task sees stop
    //    before any refused call; then draining the record pool releases
    //    every task waiting on it (their DHT work is pool calls), so the
    //    scope stops at once. The calls in flight run on (plan C7.6g).
    if let Some(scope) = state_helpers::login_scope(state) {
        scope.token().cancel();
    }
    services::record_pool::drain(state);
    stop_scope(state).await;
    // 2. Audio devices (the loops stopped with the scope).
    shutdown_voice(state, &VoiceShutdownOpts::FULL).await;
    *state.pre_away_status.write() = None;
    // 3. Work that still needs the identity's keys and records.
    services::record_pool::admit_teardown(state);
    // Only a session that opened its profile can have published itself
    // online; before that the network still holds the last logout's
    // Offline, and the record is not writable here.
    let profile_open = state
        .node
        .read()
        .as_ref()
        .is_some_and(|nh| nh.profile_lease.is_some());
    if profile_open && state_helpers::identity_status(state) != Some(UserStatus::Offline) {
        match tokio::time::timeout(
            OFFLINE_WRITE_WAIT,
            services::presence_service::publish_status(state, UserStatus::Offline),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "failed to publish offline status"),
            Err(_) => tracing::info!("offline status still publishing; logout continues"),
        }
    }
    // 4. The record pool: after the last write above, before cleanup.
    let unsent = services::record_pool::end(state).await;
    if unsent > 0 {
        if let Some(app) = app {
            crate::event_dispatch::emit_notification(
                app,
                rekindle_types::subscription_events::NotificationEvent::SystemAlert {
                    title: "Changes still to publish".into(),
                    body: format!(
                        "{unsent} profile or contact change(s) did not reach the network before \
                         you logged out. They will be published the next time you log in."
                    ),
                },
            );
        }
    }
    // 5. Routes, flushes and in-memory state.
    services::veilid::logout_cleanup(app, state).await;
    // 6. The keys, last: everything above may still sign or decrypt.
    keystore.lock().take();
}

/// Shut the login scope down and wait for its tasks. On its own this is
/// the cleanup for a login that failed before its session was
/// established (wrong passphrase, unreadable identity): it only stops
/// what the attempt spawned.
pub async fn stop_scope(state: &AppState) {
    let scope = state.login_scope.write().take();
    if let Some(scope) = scope {
        if let Err(stuck) = scope.shutdown(SCOPE_STOP_DEADLINE).await {
            tracing::warn!(%stuck, "login scope did not stop in time");
        }
    }
}
