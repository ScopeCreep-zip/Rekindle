//! Phase 13 — thin facade for inbound DM dispatch.
//!
//! All business logic moved to `rekindle_dm::ingest::*`. This file
//! preserves the existing function signatures used by
//! `message_service::handle_incoming_message` so the dispatcher arms
//! don't need to change shape. Each function builds a `DmAdapter` and
//! delegates.

use std::sync::Arc;

use crate::db::DbPool;
use crate::services::dm_adapter::DmAdapter;
use crate::state::AppState;

#[allow(clippy::too_many_arguments, reason = "thin facade preserving message_service dispatcher arm signature")]
pub async fn handle_incoming_dm_invite(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    record_key: &str,
    slot_seed: &[u8],
    alice_pseudonym: &str,
    alice_subkey: u32,
    bob_subkey: u32,
) -> Result<(), String> {
    let adapter = DmAdapter::new(Arc::clone(state), app_handle.clone(), pool.clone());
    rekindle_dm::handle_incoming_dm_invite(
        &*adapter,
        sender_hex,
        record_key,
        slot_seed,
        alice_pseudonym,
        alice_subkey,
        bob_subkey,
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn handle_incoming_dm_decline(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
) -> Result<(), String> {
    let app_handle = crate::state_helpers::app_handle(state)
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let adapter = DmAdapter::new(Arc::clone(state), app_handle, pool.clone());
    rekindle_dm::handle_incoming_dm_decline(&*adapter, record_key)
        .await
        .map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments, reason = "thin facade preserving message_service dispatcher arm signature")]
pub async fn handle_incoming_group_dm_invite(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
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
        sender_hex,
        record_key,
        slot_seed,
        initiator_pseudonym,
        participants_json,
        wrapped_mek,
        mek_generation,
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn handle_incoming_dm_leave(
    state: &Arc<AppState>,
    pool: &DbPool,
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
