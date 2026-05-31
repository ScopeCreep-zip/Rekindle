use tauri::State;

use crate::db::DbPool;
use crate::services::community_pins_runtime::{
    get_channel_pins_inner, pin_message_inner, unpin_message_inner,
};
use crate::state::SharedState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::require_permission;
use crate::services::community_pins_runtime::PinnedMessageInfoDto;

/// Add a reaction to a community channel message.
#[tauri::command]
pub async fn add_reaction(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::SEND_MESSAGES)?;
    crate::services::community::persist_reaction(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &emoji,
        true,
    )
    .await
}

/// Remove a reaction from a community channel message.
#[tauri::command]
pub async fn remove_reaction(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::SEND_MESSAGES)?;
    crate::services::community::persist_reaction(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &emoji,
        false,
    )
    .await
}

/// Pin a message in a community channel.
#[tauri::command]
pub async fn pin_message(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    message_id: String,
) -> Result<(), String> {
    pin_message_inner(state.inner(), &community_id, channel_id, message_id)
}

/// Unpin a message from a community channel.
#[tauri::command]
pub async fn unpin_message(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    message_id: String,
) -> Result<(), String> {
    unpin_message_inner(state.inner(), &community_id, channel_id, message_id)
}

/// Get pinned messages for a community channel.
#[tauri::command]
pub async fn get_channel_pins(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<PinnedMessageInfoDto>, String> {
    get_channel_pins_inner(state.inner(), pool.inner(), community_id, channel_id).await
}
