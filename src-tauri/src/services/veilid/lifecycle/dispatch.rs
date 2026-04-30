use std::sync::Arc;

use crate::state::AppState;
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

/// Start the Veilid event dispatch loop.
pub async fn start_dispatch_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("veilid dispatch loop started");

    loop {
        tokio::select! {
            Some(update) = update_rx.recv() => {
                crate::services::veilid::handle_veilid_update(&app_handle, &state, update).await;
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("veilid dispatch loop shutting down");
                break;
            }
        }
    }
}
