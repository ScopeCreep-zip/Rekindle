//! Phase 13 — thin facade for DM message send/receive.
//!
//! All business logic moved to `rekindle_dm::sender::send_dm_message` +
//! `rekindle_dm::receiver::handle_dm_subkey_change`. This file is the
//! src-tauri entry point: pulls the `AppHandle` off `AppState`,
//! constructs a `DmAdapter`, and delegates.
//!
//! Callsites in `commands/dm.rs` + `services/veilid/dht_watch.rs`
//! continue to use these function signatures without changes.

use std::sync::Arc;

use crate::db::DbPool;
use crate::services::dm_adapter::DmAdapter;
use crate::state::AppState;
use crate::state_helpers;

/// Outbound 1:1 DM send. Errors map to Tauri-friendly `String`.
pub async fn send_dm_message(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
    body: &str,
) -> Result<(), String> {
    let app_handle =
        state_helpers::app_handle(state).ok_or_else(|| "app handle unavailable".to_string())?;
    let adapter = DmAdapter::new(Arc::clone(state), app_handle, pool.clone());
    rekindle_dm::send_dm_message(&*adapter, record_key, body)
        .await
        .map_err(|e| e.to_string())
}

/// Inbound DM subkey-changed handler. Called from
/// `services/veilid/dht_watch::handle_value_change` when a watch fires
/// on a DM SMPL record.
pub async fn handle_dm_subkey_change(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
    subkey: u32,
    raw_value: &[u8],
) -> Result<(), String> {
    let app_handle =
        state_helpers::app_handle(state).ok_or_else(|| "app handle unavailable".to_string())?;
    let adapter = DmAdapter::new(Arc::clone(state), app_handle, pool.clone());
    rekindle_dm::handle_dm_subkey_change(&*adapter, record_key, subkey, raw_value)
        .await
        .map_err(|e| e.to_string())
}
