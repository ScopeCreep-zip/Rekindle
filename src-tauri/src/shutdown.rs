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
/// (0.5.8 `storage_manager/mod.rs:635-678`); a fanout has no cancellation API
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
    // Managed at setup, before the Host starts; absent only if setup
    // failed first, and then there is no Host to have wiped anything.
    let root = app_handle
        .try_state::<rekindle_db::paths::DataRoot>()
        .map(|root| root.inner().clone());
    tauri::async_runtime::block_on(async move {
        let started = std::time::Instant::now();
        graceful_shutdown(&state, root.as_ref()).await;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "graceful shutdown finished"
        );
    });
}

/// The Host wiped the data root (`StreamFrame::HostStopping { DataWiped }`,
/// plan E7.1): clear every webview's browsing data, then quit. The quit
/// runs [`graceful_shutdown`], which finishes the wipe once the node has
/// stopped and removes the webview directories. Nothing restarts the app.
pub fn exit_after_wipe(app: &tauri::AppHandle) {
    for (label, window) in app.webview_windows() {
        if let Err(e) = window.clear_all_browsing_data() {
            tracing::error!(window = %label, error = %e, "webview data not cleared");
        }
    }
    app.exit(0);
}

/// Shut down all background services before the process exits: the Host
/// (its bus, then the session's teardown with `EndReason::Exit`), then the
/// app-lifetime dispatch loop and the Veilid node. After a wipe, the rest
/// of the data root and the webview stores go once the node has stopped.
pub async fn graceful_shutdown(state: &SharedState, root: Option<&rekindle_db::paths::DataRoot>) {
    tracing::info!("graceful shutdown: stopping background services");

    // 1. The in-process host: its bus drains (every request in flight is
    //    answered), then it ends the session (its tasks, the Offline
    //    status, the records, our routes, the keys) before the node it
    //    serves over stops.
    let mut wiped = false;
    if let Some(host) = state.host.current() {
        match host.shutdown().await {
            Ok(reason) => {
                wiped = reason == rekindle_node::daemon::shutdown::ExitReason::DataWiped;
            }
            Err(e) => tracing::warn!(error = %format!("{e:#}"), "host shutdown"),
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

    // 4. After a wipe: what the node held, then the webview stores outside
    //    the data root (the shell's node is the GUI's, so the Host left
    //    these to it; `rekindle_node::host::wipe`).
    if let (true, Some(root)) = (wiped, root) {
        let finished = rekindle_node::host::wipe::finish(root)
            .map_err(|e| e.to_string())
            .and_then(|()| rekindle_db::paths::DataRoot::webview_dirs().map_err(|e| e.to_string()))
            .and_then(|dirs| {
                rekindle_node::host::wipe::remove_paths(&dirs).map_err(|e| e.to_string())
            });
        match finished {
            Ok(()) => tracing::info!("data root and webview stores wiped"),
            Err(e) => tracing::error!(error = %e, "wipe incomplete"),
        }
    }

    tracing::info!("graceful shutdown complete");
}
