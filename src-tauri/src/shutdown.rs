//! Graceful application shutdown.

use tauri::Manager;

use crate::state::{self, SharedState, UserStatus};
use crate::{commands, db, services, state_helpers};

/// Handle the `RunEvent::Exit` event: run [`graceful_shutdown`] with a timeout
/// and checkpoint the SQLite WAL before the process exits.
pub fn handle_exit(app_handle: &tauri::AppHandle) {
    // app.exit(0) was called (from tray quit or system shutdown).
    // Run graceful shutdown with a timeout to prevent hanging.
    tracing::info!("RunEvent::Exit fired — starting graceful shutdown");
    let state: tauri::State<'_, SharedState> = app_handle.state();
    let state = state.inner().clone();
    let pool: tauri::State<'_, db::DbPool> = app_handle.state();
    let pool = pool.inner().clone();
    tauri::async_runtime::block_on(async move {
        let shutdown = graceful_shutdown(&state);
        if tokio::time::timeout(std::time::Duration::from_secs(5), shutdown)
            .await
            .is_err()
        {
            tracing::warn!("graceful shutdown timed out after 5s — forcing exit");
        }
        // Checkpoint WAL to prevent Windows NTFS file lock issues.
        // tokio_rusqlite::Connection drops gracefully after this.
        let _: Result<(), tokio_rusqlite::Error<rusqlite::Error>> = pool
            .call(|conn| {
                conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
                Ok(())
            })
            .await;
    });
}

/// Shut down all background services before the process exits.
///
/// First cleans up user-specific state (DHT records, routes), then sends
/// shutdown signals to the dispatch loop, and finally shuts down the
/// Veilid node itself.
pub async fn graceful_shutdown(state: &SharedState) {
    tracing::info!("graceful shutdown: stopping background services");

    // 1. Send graceful shutdown signals to all services FIRST.
    //    This gives them a chance to finish their current operation before
    //    logout_cleanup aborts any remaining handles.

    // Signal sync service shutdown
    let sync_tx = state.sync_shutdown_tx.read().clone();
    if let Some(tx) = sync_tx {
        let _ = tx.send(()).await;
    }

    // Signal game detection shutdown
    let game_tx = state
        .game_detector
        .lock()
        .as_ref()
        .map(|h| h.shutdown_tx.clone());
    if let Some(tx) = game_tx {
        let _ = tx.send(()).await;
    }

    // Signal route refresh loop shutdown
    let route_watchdog_tx = state.route_watchdog_shutdown_tx.write().take();
    if let Some(tx) = route_watchdog_tx {
        let _ = tx.send(()).await;
    }

    // Signal idle service shutdown
    let idle_tx = state.idle_shutdown_tx.write().take();
    if let Some(tx) = idle_tx {
        let _ = tx.send(()).await;
    }

    // Signal heartbeat shutdown
    let heartbeat_tx = state.heartbeat_shutdown_tx.write().take();
    if let Some(tx) = heartbeat_tx {
        let _ = tx.send(()).await;
    }

    // Signal dispatch loop shutdown
    let shutdown_tx = state.shutdown_tx.read().clone();
    if let Some(tx) = shutdown_tx {
        let _ = tx.send(()).await;
    }

    // 2. Shut down voice engine (signal loops, await, then stop devices)
    commands::voice::shutdown_voice(state, &commands::voice::VoiceShutdownOpts::FULL).await;

    // 3. Await the dispatch loop handle (it should have exited after the shutdown signal)
    {
        let dispatch_handle = state.dispatch_loop_handle.write().take();
        if let Some(h) = dispatch_handle {
            let _ = h.await;
        }
    }

    // 6. Publish Offline to DHT before cleanup closes records
    {
        let current_status = state_helpers::identity_status(state);
        if current_status != Some(state::UserStatus::Offline) {
            if let Err(e) =
                services::presence_service::publish_status(state, UserStatus::Offline).await
            {
                tracing::warn!(error = %e, "failed to publish offline on shutdown");
            }
        }
    }

    // 7. Now clean up user-specific DHT state (close records, release route,
    //    abort remaining background handles).
    //    Pass None for app_handle — the app is exiting, no UI to update.
    services::veilid::logout_cleanup(None, state).await;

    // Clear community state
    state.mek_cache.lock().clear();

    // 8. Shut down the Veilid node (only on app exit)
    services::veilid::shutdown_app(state).await;

    tracing::info!("graceful shutdown complete");
}
