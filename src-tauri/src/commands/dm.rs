//! Tauri commands for direct messages (architecture §27).
//!
//! The wire-level orchestration (creating SMPL records, sending invites,
//! deriving MEKs) lives in `services/dm/`. These commands are thin
//! adapters for the frontend.

use tauri::State;

use crate::db::DbPool;
use crate::services::dm;
use crate::state::SharedState;

#[tauri::command]
pub async fn list_dms(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<dm::store::DmConversation>, String> {
    Ok(dm::list_dm_conversations(state.inner(), pool.inner()).await)
}

#[tauri::command]
pub async fn start_dm(
    bob_public_key: String,
    alice_pseudonym: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    dm::start_dm(
        state.inner(),
        pool.inner(),
        &bob_public_key,
        &alice_pseudonym,
    )
    .await
}

#[tauri::command]
pub async fn accept_dm_invite(
    record_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    dm::accept_dm_invite(state.inner(), pool.inner(), &record_key).await
}

#[tauri::command]
pub async fn decline_dm_invite(
    record_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    dm::decline_dm_invite(state.inner(), pool.inner(), &record_key).await
}

#[tauri::command]
pub async fn send_dm_message(
    record_key: String,
    body: String,
    idempotency_key: uuid::Uuid,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Phase 5 — gate writes on lifecycle.
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    // Phase 8 — idempotency dedupes click-spam.
    let state_for_cache = state.inner().clone();
    let pool_for_cache = pool.inner().clone();
    state
        .idempotency
        .wrap(idempotency_key, || async move {
            dm::send_dm_message(&state_for_cache, &pool_for_cache, &record_key, &body).await
        })
        .await
}

#[tauri::command]
pub async fn get_dm_messages(
    record_key: String,
    limit: Option<i64>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<dm::store::DmMessageRecord>, String> {
    let limit = limit.unwrap_or(200);
    Ok(dm::load_dm_messages(state.inner(), pool.inner(), &record_key, limit).await)
}

/// W11.4 (P6.2) — send one encoded video frame to a 1:1 DM peer.
///
/// Mirrors `commands::community::video::send_video_frame` but routes
/// 1:1 via the Signal-encrypted `send_envelope_to_peer` path instead
/// of mesh fan-out, and uses no community MEK (Signal Double Ratchet
/// already covers confidentiality + authenticity).
///
/// The frontend produces VP9 chunks via WebCodecs and base64-encodes
/// them for IPC; the backend chunks ≤28 KB, wraps each chunk in a
/// `DmVideoFragment` payload, and sends through the existing DM
/// transport. Returns the number of fragments dispatched.
#[tauri::command]
pub async fn send_dm_video_frame(
    peer_pubkey: String,
    request: SendDmVideoFrameRequest,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<u32, String> {
    crate::services::dm_runtime::send_dm_video_frame_inner(
        state.inner(),
        pool.inner(),
        peer_pubkey,
        request,
    )
    .await
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendDmVideoFrameRequest {
    pub stream_id_hex: String,
    pub frame_seq: u32,
    pub keyframe: bool,
    pub timestamp: u32,
    pub encoded_payload_b64: String,
}
