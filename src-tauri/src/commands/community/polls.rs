use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::require_permission;

#[tauri::command]
pub async fn create_poll(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    message_id: String,
    question: String,
    answers: Vec<String>,
    multi_select: bool,
    expires_at: Option<u64>,
) -> Result<String, String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::SEND_MESSAGES)?;
    crate::services::community::persist_poll_create(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &question,
        answers,
        multi_select,
        expires_at,
    )
    .await
}

#[tauri::command]
pub async fn vote_poll(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    poll_id: String,
    selected_answers: Vec<u8>,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::SEND_MESSAGES)?;
    crate::services::community::persist_poll_vote(
        state.inner(),
        &community_id,
        &channel_id,
        &poll_id,
        selected_answers,
    )
    .await
}

#[tauri::command]
pub async fn close_poll(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    poll_id: String,
) -> Result<(), String> {
    let _ = pool;
    let moderator_override =
        require_permission(state.inner(), &community_id, Permissions::MANAGE_MESSAGES).is_ok();
    crate::services::community::persist_poll_close(
        state.inner(),
        &community_id,
        &channel_id,
        &poll_id,
        moderator_override,
    )
    .await
}
