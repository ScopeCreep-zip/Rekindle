//! Phase 21 REDO — thin facade.
//!
//! Friend-presence orchestrators (DHT value-change dispatch +
//! `watch_friend`) live in `rekindle_presence::friend`. This module
//! constructs a `PresenceAdapter` per call and delegates. Our own status
//! is the Host's (`host::own_presence`, plan E1.1).

use std::sync::Arc;

use crate::services::presence_adapter::build_adapter;
use crate::state::AppState;

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
