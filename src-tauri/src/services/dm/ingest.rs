//! Phase 13 — thin facade for inbound group DM dispatch.
//!
//! All business logic is in `rekindle_dm::ingest::*`; the dispatcher arms
//! in `message_service::dm_dispatch` call these. Each function builds a
//! `DmAdapter` and delegates.

use std::sync::Arc;

use crate::services::dm_adapter::DmAdapter;
use crate::state::AppState;
use rekindle_db::Db;

pub async fn handle_incoming_group_dm_invite(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    sender_hex: &str,
    record_key: &str,
    slot_seed: &[u8],
    initiator_pseudonym: &str,
    participants_json: &str,
    wrapped_mek: &[u8],
    mek_generation: u32,
) -> Result<(), String> {
    let adapter = DmAdapter::new(Arc::clone(state), app_handle.clone(), pool.clone());
    rekindle_dm::handle_incoming_group_dm_invite(
        &*adapter,
        rekindle_dm::IncomingGroupDmInvite {
            sender_hex,
            record_key,
            slot_seed,
            initiator_pseudonym,
            participants_json,
            wrapped_mek,
            mek_generation,
        },
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn handle_incoming_dm_leave(
    state: &Arc<AppState>,
    pool: &Db,
    sender_hex: &str,
    record_key: &str,
) -> Result<(), String> {
    let app_handle = crate::state_helpers::app_handle(state)
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let adapter = DmAdapter::new(Arc::clone(state), app_handle, pool.clone());
    rekindle_dm::handle_incoming_dm_leave(&*adapter, sender_hex, record_key)
        .await
        .map_err(|e| e.to_string())
}
