//! Phase 21 REDO — thin facade.
//!
//! Friend-presence orchestrators (DHT value-change dispatch +
//! `watch_friend` + `publish_status` + heartbeat loop) live in
//! `rekindle_presence::{friend, heartbeat}`. This module constructs a
//! `PresenceAdapter` per call and delegates.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::services::presence_adapter::build_adapter;
use crate::state::{AppState, UserStatus};

/// Map src-tauri's [`UserStatus`] to the crate's wire-clean enum.
fn to_crate_status(status: UserStatus) -> rekindle_presence::UserStatusKind {
    match status {
        UserStatus::Online => rekindle_presence::UserStatusKind::Online,
        UserStatus::Away => rekindle_presence::UserStatusKind::Away,
        UserStatus::Busy => rekindle_presence::UserStatusKind::Busy,
        UserStatus::Offline => rekindle_presence::UserStatusKind::Offline,
        UserStatus::Invisible => rekindle_presence::UserStatusKind::Invisible,
    }
}

/// Handle a DHT value change for a watched friend (or a community
/// record — falls through to a debug trace today; future Phase 21
/// REDO sub-step will move community presence into the crate too).
pub fn handle_value_change(
    _app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    dht_key: &str,
    subkeys: &[u32],
    value: &[u8],
) {
    let Some(adapter) = build_adapter(state) else {
        tracing::debug!(dht_key, "handle_value_change: adapter unavailable");
        return;
    };
    rekindle_presence::handle_value_change(&adapter, dht_key, subkeys, value);
}

/// Subscribe to a friend's DHT presence record.
pub async fn watch_friend(
    state: &Arc<AppState>,
    friend_key: &str,
    dht_record_key: &str,
) -> Result<(), String> {
    let Some(adapter) = build_adapter(state) else {
        return Err("app handle / db pool unavailable".to_string());
    };
    let adapter = Arc::new(adapter);
    rekindle_presence::watch_friend(adapter, friend_key, dht_record_key)
        .await
        .map_err(|e| e.to_string())
}

/// Publish our own status to profile subkey 2.
pub async fn publish_status(state: &Arc<AppState>, status: UserStatus) -> Result<(), String> {
    let Some(adapter) = build_adapter(state) else {
        return Err("app handle / db pool unavailable".to_string());
    };
    let adapter = Arc::new(adapter);
    rekindle_presence::publish_status(adapter, to_crate_status(status))
        .await
        .map_err(|e| e.to_string())
}

/// Run the periodic status heartbeat loop until `shutdown_rx` fires.
pub async fn start_heartbeat_loop(state: Arc<AppState>, shutdown_rx: mpsc::Receiver<()>) {
    let Some(adapter) = build_adapter(&state) else {
        tracing::warn!("start_heartbeat_loop: adapter unavailable — heartbeat skipped");
        return;
    };
    let adapter = Arc::new(adapter);
    rekindle_presence::start_heartbeat_loop(adapter, shutdown_rx).await;
}
