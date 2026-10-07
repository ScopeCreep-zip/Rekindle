//! Graceful application shutdown.

use tauri::Manager;

use crate::keystore::KeystoreHandle;
use crate::state::SharedState;
use crate::{services, state_helpers};

/// Handle the `RunEvent::Exit` event: run [`graceful_shutdown`] with a timeout
/// and checkpoint the SQLite WAL before the process exits.
pub fn handle_exit(app_handle: &tauri::AppHandle) {
    // app.exit(0) was called (from tray quit or system shutdown).
    // Run graceful shutdown with a timeout to prevent hanging.
    tracing::info!("RunEvent::Exit fired — starting graceful shutdown");
    let state: tauri::State<'_, SharedState> = app_handle.state();
    let state = state.inner().clone();
    let keystore: tauri::State<'_, KeystoreHandle> = app_handle.state();
    let keystore = keystore.inner().clone();
    tauri::async_runtime::block_on(async move {
        let shutdown = graceful_shutdown(&state, &keystore);
        if tokio::time::timeout(std::time::Duration::from_secs(5), shutdown)
            .await
            .is_err()
        {
            tracing::warn!("graceful shutdown timed out after 5s — forcing exit");
        }
        // Checkpoint the WAL (Windows NTFS holds file locks on it), then
        // close the database.
        if let Ok(pool) = state.db.current() {
            if let Err(e) = pool
                .call(|conn| conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)"))
                .await
            {
                tracing::warn!(error = %e, "WAL checkpoint at exit failed");
            }
        }
        if let Err(e) = state.db.clear().await {
            tracing::warn!(error = %e, "database still held at exit");
        }
    });
}

/// Shut down all background services before the process exits: end the
/// login session (`services::session::end_session`), then stop the
/// app-lifetime dispatch loop and the Veilid node.
pub async fn graceful_shutdown(state: &SharedState, keystore: &KeystoreHandle) {
    tracing::info!("graceful shutdown: stopping background services");

    // 1. The login session: its tasks, devices, offline status, records,
    //    state and keys. No app handle — the app is exiting.
    services::session::end_session(None, state, keystore).await;
    state_helpers::clear_meks(state);

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
