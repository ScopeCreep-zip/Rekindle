use std::sync::Arc;

use crate::state::AppState;
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

/// Phase 9 — drain the cold-start buffer through `handle_veilid_update`.
/// Called once on the first lifecycle Operational transition. After the
/// drain, subsequent identity-dependent events bypass the buffer
/// (`try_record` returns `Err`) and dispatch directly in the main loop.
async fn drain_cold_start(app_handle: &tauri::AppHandle, state: &Arc<AppState>) {
    let mut drained: Vec<VeilidUpdate> = Vec::new();
    state.cold_start.install_callback(|ev| drained.push(ev));
    if drained.is_empty() {
        return;
    }
    tracing::info!(count = drained.len(), "cold_start_drain");
    for ev in drained {
        crate::services::veilid::handle_veilid_update(app_handle, state, ev).await;
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
    let (worker_stop_tx, mut worker_stop_rx) = tokio::sync::watch::channel(false);
    let worker_handle = {
        let app_handle = app_handle.clone();
        let state = Arc::clone(&state);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = worker_stop_rx.changed() => {
                        if *worker_stop_rx.borrow() {
                            tracing::info!("gossip ingress worker shutting down");
                            break;
                        }
                    }
                    item = state.gossip_ingress.pop() => {
                        crate::services::veilid::app_message::process_ingress_item(
                            &app_handle,
                            &state,
                            item,
                        )
                        .await;
                    }
                }
            }
        })
    };

    // Piece 6 — control ingress worker: sibling to the gossip worker.
    // The serial recv loop used to `.await` the SLOW identity-dependent
    // handlers inline — `ValueChange` (DHT read → DB → decrypt) and
    // `AppCall` (network round-trips). While it did, incoming media
    // `AppMessage`s backed up in the node's 4096-deep update channel and
    // overflowed (the measured 388 media drops). This worker drains
    // `ValueChange`/`AppCall` off-loop via `handle_veilid_update` so the
    // recv loop returns immediately. Single worker → FIFO among control
    // events (largely order-tolerant, but kept ordered conservatively).
    // Shares the gossip worker's stop signal (one `send(true)` stops
    // both).
    let mut control_stop_rx = worker_stop_tx.subscribe();
    let control_worker_handle = {
        let app_handle = app_handle.clone();
        let state = Arc::clone(&state);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = control_stop_rx.changed() => {
                        if *control_stop_rx.borrow() {
                            tracing::info!("control ingress worker shutting down");
                            break;
                        }
                    }
                    item = state.control_ingress.pop() => {
                        crate::services::veilid::handle_veilid_update(
                            &app_handle,
                            &state,
                            item,
                        )
                        .await;
                    }
                }
            }
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
        drain_cold_start(&app_handle, &state).await;
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
                            drain_cold_start(&app_handle, &state).await;
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
                            drain_cold_start(&app_handle, &state).await;
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
                match state.cold_start.try_record(update) {
                    Ok(()) => {
                        // Buffered for later drain. Nothing to do now.
                    }
                    Err(update) => {
                        // Piece 6 — post-cold-start, offload the SLOW
                        // identity-dependent handlers to the control
                        // ingress worker so this serial recv loop returns
                        // immediately and the node's 4096-deep update
                        // channel can't back up incoming media
                        // `AppMessage`s behind them (the measured 388
                        // media drops). `AppMessage` STAYS inline: its
                        // handler is already sync-fast (media `try_send` /
                        // gossip-queue push, returns immediately).
                        //
                        // Ordering is safe: media (`AppMessage`) and
                        // control (`ValueChange`/`AppCall`) are independent
                        // inbound streams with no cross-ordering
                        // dependency, so decoupling them reorders nothing
                        // that matters. The single control worker preserves
                        // FIFO among control events; governance
                        // `ValueChange`s are CRDT-merged (order-independent)
                        // and presence `ValueChange`s are staleness-checked
                        // via `last_heartbeat`.
                        match update {
                            VeilidUpdate::ValueChange(_) | VeilidUpdate::AppCall(_) => {
                                state.control_ingress.push(update);
                            }
                            _ => {
                                crate::services::veilid::handle_veilid_update(
                                    &app_handle,
                                    &state,
                                    update,
                                )
                                .await;
                            }
                        }
                    }
                }
            }
        }
    }
}
