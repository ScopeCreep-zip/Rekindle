//! Graceful application shutdown.

use tauri::Manager;

use crate::services;
use crate::state::SharedState;

/// Handle the `RunEvent::Exit` event: run [`graceful_shutdown`] to the end
/// (the Host's teardown checkpoints and closes the identity database).
///
/// Not under a timeout. The session teardown is bounded by its own
/// deadlines (`SESSION_STOP_DEADLINE`, the Offline write wait). Veilid's
/// shutdown waits for its in-flight DHT calls (`startup_lock.shutdown()`),
/// then flushes both record stores and saves its metadata
/// (`storage_manager/mod.rs:509-550`); a fanout has no cancellation API
/// (veilid #516) and can outlast `LONGEST_VEILID_CALL`. Cutting it off,
/// as a 5 s cap did, skipped that flush: a write already on the network
/// was missing locally after restart (plan C7.16), and an abort mid-commit
/// wedges the record store (C7.6g). The daemon awaits it the same way
/// (`TransportNode::shutdown`).
pub fn handle_exit(app_handle: &tauri::AppHandle) {
    // app.exit(0) was called (from tray quit or system shutdown).
    tracing::info!("RunEvent::Exit fired — starting graceful shutdown");
    let state: tauri::State<'_, SharedState> = app_handle.state();
    let state = state.inner().clone();
    tauri::async_runtime::block_on(async move {
        let started = std::time::Instant::now();
        graceful_shutdown(&state).await;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "graceful shutdown finished"
        );
    });
}

/// Shut down all background services before the process exits: the Host
/// (its bus, then the session's teardown with `EndReason::Exit`), then the
/// app-lifetime dispatch loop and the Veilid node.
pub async fn graceful_shutdown(state: &SharedState) {
    tracing::info!("graceful shutdown: stopping background services");

    // 1. The in-process host: its bus drains (every request in flight is
    //    answered), then it ends the session (its tasks, the Offline
    //    status, the records, our routes, the keys) before the node it
    //    serves over stops.
    if let Some(host) = state.host.current() {
        if let Err(e) = host.shutdown().await {
            tracing::warn!(error = %format!("{e:#}"), "host shutdown");
        }
    }

    // 2. The app-lifetime dispatch loop.
    let shutdown_tx = state.shutdown_tx.read().clone();
    if let Some(tx) = shutdown_tx {
        let _ = tx.send(()).await;
    }
    let dispatch_handle = state.dispatch_loop_handle.write().take();
    if let Some(h) = dispatch_handle {
        let _ = h.await;
    }

    // 3. The Veilid node itself.
    services::veilid::shutdown_app(state).await;

    tracing::info!("graceful shutdown complete");
}
