//! Phase 19.i-REDO — thin facade.
//!
//! `persist_reaction`'s full body (reaction_write_context + DHT write
//! + gossip envelope + dual-path success) lives in
//! `rekindle_channel::reactions::persist_reaction`. This module is a
//! 1-call delegation.

use std::sync::Arc;

use tauri::Manager;

use crate::state::AppState;

pub async fn persist_reaction(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    expression: &str,
    added: bool,
) -> Result<(), String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<crate::db::DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    let adapter =
        crate::services::channel_adapter::ChannelAdapter::new(Arc::clone(state), app_handle, pool);
    rekindle_channel::persist_reaction(
        &adapter,
        community_id,
        channel_id,
        message_id,
        expression,
        added,
    )
    .await
    .map_err(|e| e.to_string())
}
