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
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    dm::send_dm_message(state.inner(), pool.inner(), &record_key, &body).await
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
