//! Phase 21 REDO — periodic status heartbeat loop.
//!
//! Pre-port this lived in `src-tauri/services/presence_service.rs`
//! and read `state.identity` + `state.node` directly. Here it
//! parameterises over [`FriendPresenceDeps`] so the timer + skip
//! semantics are testable without spinning up a real node.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::interval;

use crate::deps::FriendPresenceDeps;
use crate::friend::publish_status;
use crate::status::UserStatusKind;

/// Default heartbeat cadence (60 s). Pre-port this was a magic
/// constant; lifted so tests + observers can reference the same
/// value as the runtime.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 60;

/// Run the heartbeat loop until `shutdown_rx` fires. Skips ticks
/// when the user isn't logged in or has already set their status to
/// Offline.
pub async fn start_heartbeat_loop<D: FriendPresenceDeps>(
    deps: Arc<D>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut tick = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    tick.tick().await; // skip the immediate first fire

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let status = match deps.current_identity_status() {
                    Some(s) if s != UserStatusKind::Offline => s,
                    _ => continue,
                };
                if let Err(error) = publish_status(Arc::clone(&deps), status).await {
                    tracing::debug!(%error, "heartbeat publish failed");
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::debug!("presence heartbeat loop shutting down");
                break;
            }
        }
    }
}
