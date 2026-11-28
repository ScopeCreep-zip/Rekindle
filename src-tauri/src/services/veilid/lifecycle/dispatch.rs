use std::sync::Arc;

use crate::state::AppState;
use rekindle_transport::subscriptions::lanes::work_until_stopped;
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

/// Returns true when `update` requires app state (identity, vault,
/// signal sessions) to process. These events are buffered while the
/// app is pre-`Operational` and dispatched only after the consumer
/// signals readiness via lifecycle transition.
///
/// `Attachment`, `Network`, `RouteChange`, `Config`, `Log`, `Shutdown`
/// drive the lifecycle FSM itself — they must dispatch immediately
/// or the app can never reach `Operational` to drain the buffer.
fn needs_app_state(update: &VeilidUpdate) -> bool {
    matches!(
        update,
        VeilidUpdate::AppMessage(_) | VeilidUpdate::AppCall(_) | VeilidUpdate::ValueChange(_)
    )
}

/// A short name for an update, for logs.
fn update_kind(update: &VeilidUpdate) -> &'static str {
    match update {
        VeilidUpdate::AppMessage(_) => "app_message",
        VeilidUpdate::AppCall(_) => "app_call",
        VeilidUpdate::ValueChange(_) => "value_change",
        _ => "other",
    }
}

/// Drain recorded identity-dependent updates through their normal lanes.
fn drain_cold_start(app_handle: &tauri::AppHandle, state: &Arc<AppState>) {
    let mut drained = Vec::new();
    state
        .cold_start
        .install_callback(|event| drained.push(event));
    tracing::info!(count = drained.len(), "cold_start_drain");
    for update in drained {
        dispatch_identity_update(app_handle, state, update);
    }
}

fn dispatch_identity_update(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    update: VeilidUpdate,
) {
    let Some(scope) = crate::state_helpers::login_scope(state) else {
        tracing::debug!(
            kind = update_kind(&update),
            "identity-dependent update dropped: no session"
        );
        return;
    };
    match update {
        VeilidUpdate::ValueChange(_) => state.control_ingress.push(update),
        VeilidUpdate::AppMessage(message) => super::super::app_message::handle(state, &message),
        VeilidUpdate::AppCall(_) => {
            let app_handle = app_handle.clone();
            let worker = Arc::clone(state);
            state.app_call_lane.dispatch(&scope, async move {
                crate::services::veilid::handle_veilid_update(&app_handle, &worker, update).await;
            });
        }
        _ => unreachable!("only identity-dependent updates are recorded"),
    }
}

/// Start the Veilid event dispatch loop.
pub async fn start_dispatch_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("veilid dispatch loop started");

    // Phase 3 — gossip ingress worker: drains the queue the
    // app_message callback pushes into, so video-fragment processing
    // can never stall this serial loop (which also carries the voice
    // fast path). Single worker — per-sender fragment ORDER is
    // load-bearing for reassembly. Terminated by a watch flag flipped
    // when this loop shuts down.
    let (worker_stop_tx, worker_stop_rx) = tokio::sync::watch::channel(false);
    let worker_handle = {
        let app_handle = app_handle.clone();
        let state = Arc::clone(&state);
        tauri::async_runtime::spawn(async move {
            let (app_handle, state) = (&app_handle, &state);
            work_until_stopped(
                worker_stop_rx,
                "gossip",
                move || state.gossip_ingress.pop(),
                move |item| {
                    crate::services::veilid::app_message::process_ingress_item(
                        app_handle, state, item,
                    )
                },
            )
            .await;
        })
    };

    // Watch notifications drain on one control worker so DHT reads leave
    // the receiver free to handle media. Both workers share the stop flag.
    let control_stop_rx = worker_stop_tx.subscribe();
    let control_worker_handle = {
        let app_handle = app_handle.clone();
        let state = Arc::clone(&state);
        tauri::async_runtime::spawn(async move {
            let (app_handle, state) = (&app_handle, &state);
            work_until_stopped(
                control_stop_rx,
                "control",
                move || state.control_ingress.pop(),
                move |item| async move {
                    // Queued before a logout, popped after: the update
                    // belongs to a session that has ended (plan C4.L3).
                    if crate::state_helpers::login_scope(state).is_none() {
                        tracing::debug!("identity-dependent update dropped: no session");
                        return;
                    }
                    crate::services::veilid::handle_veilid_update(app_handle, state, item).await;
                },
            )
            .await;
        })
    };

    // Phase 9 — listen for lifecycle transitions so the cold-start
    // buffer can drain the moment the app becomes Operational. The
    // current() check below covers the (unlikely) case where Operational
    // was reached before this subscriber was created.
    let mut lifecycle_rx = state.lifecycle.subscribe();
    if state.lifecycle.current() == rekindle_lifecycle::LifecycleState::Operational
        && !state.cold_start.is_installed()
    {
        drain_cold_start(&app_handle, &state);
    }

    loop {
        tokio::select! {
            biased;
            _ = shutdown_rx.recv() => {
                tracing::info!("veilid dispatch loop shutting down");
                let _ = worker_stop_tx.send(true);
                worker_handle.abort();
                control_worker_handle.abort();
                break;
            }
            lifecycle_result = lifecycle_rx.recv() => {
                match lifecycle_result {
                    Ok(new_state) => {
                        if new_state == rekindle_lifecycle::LifecycleState::Operational
                            && !state.cold_start.is_installed()
                        {
                            drain_cold_start(&app_handle, &state);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "lifecycle broadcast lagged in dispatch loop");
                        // Defensive: a lag could have swallowed the
                        // Operational transition that triggers the
                        // cold-start drain. Re-read current state and
                        // drain if we're now Operational. Without this,
                        // a lag here would strand every buffered event
                        // for the rest of the app session.
                        if state.lifecycle.current()
                            == rekindle_lifecycle::LifecycleState::Operational
                            && !state.cold_start.is_installed()
                        {
                            drain_cold_start(&app_handle, &state);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Lifecycle channel closed — keep dispatching;
                        // remaining events will go through try_record's
                        // Err path until shutdown signals.
                    }
                }
            }
            Some(update) = update_rx.recv() => {
                // Lifecycle-independent events always dispatch immediately.
                if !needs_app_state(&update) {
                    crate::services::veilid::handle_veilid_update(&app_handle, &state, update).await;
                    continue;
                }
                // Identity-dependent: buffer until consumer install, then
                // dispatch once the buffer is closed.
                if let Err(update) = state.cold_start.try_record(update) {
                    dispatch_identity_update(&app_handle, &state, update);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
