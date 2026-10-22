use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::commands::chat::Message;
use crate::state::SharedState;

/// Forward our stored copy of a channel message into another channel
/// (`CommunityRequest::ChannelMessageForward`); the source and destination
/// may be in different communities.
#[tauri::command]
pub async fn forward_channel_message(
    source_community_id: String,
    source_channel_id: String,
    source_message_id: String,
    dest_community_id: String,
    dest_channel_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::ChannelMessageForward {
        source_community: source_community_id,
        source_channel: source_channel_id,
        source_message: source_message_id,
        dest_community: dest_community_id,
        dest_channel: dest_channel_id,
    });
    crate::host_call::call(&state, request).await
}

/// Post a message in a community channel
/// (`CommunityRequest::ChannelMessageSend`).
#[tauri::command]
pub async fn send_channel_message(
    channel_id: String,
    body: String,
    reply_to_id: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::ChannelMessageSend {
        channel: channel_id,
        body,
        reply_to: reply_to_id,
    });
    crate::host_call::call(&state, request).await
}

/// Replace the body of our message in a channel
/// (`CommunityRequest::ChannelMessageEdit`).
#[tauri::command]
pub async fn edit_channel_message(
    channel_id: String,
    message_id: String,
    new_body: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::ChannelMessageEdit {
        channel: channel_id,
        message_id,
        body: new_body,
    });
    crate::host_call::call(&state, request).await
}

/// Delete our message in a channel (`CommunityRequest::ChannelMessageDelete`).
#[tauri::command]
pub async fn delete_channel_message(
    channel_id: String,
    message_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::ChannelMessageDelete {
        channel: channel_id,
        message_id,
    });
    crate::host_call::call(&state, request).await
}

#[tauri::command]
pub async fn get_channel_messages(
    channel_id: String,
    limit: u32,
    state: State<'_, SharedState>,
) -> Result<Vec<Message>, String> {
    let pool = state.db.current()?;
    crate::services::messaging_runtime::get_channel_messages_inner(
        state.inner().clone(),
        pool.clone(),
        channel_id,
        limit,
    )
    .await
}

#[tauri::command]
pub async fn get_older_channel_messages(
    community_id: String,
    channel_id: String,
    before_timestamp: u64,
    limit: u32,
    state: State<'_, SharedState>,
) -> Result<Vec<Message>, String> {
    let pool = state.db.current()?;
    crate::services::messaging_runtime::get_older_channel_messages_inner(
        state.inner().clone(),
        pool.clone(),
        community_id,
        channel_id,
        before_timestamp,
        limit,
    )
    .await
}
