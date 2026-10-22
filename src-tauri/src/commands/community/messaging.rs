use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::commands::chat::Message;
use crate::services::messaging_runtime::SendChannelMessageResponse;
use crate::state::SharedState;

/// Forward a previously-cached channel message to another channel.
///
/// `source_*` and `dest_*` may identify the same community or different
/// communities. The source must already be cached locally (no DHT refetch).
/// Re-encrypts content with the destination MEK and writes a
/// `ChannelEntry::Forward` entry plus a gossip notification.
#[tauri::command]
pub async fn forward_channel_message(
    source_community_id: String,
    source_channel_id: String,
    source_message_id: String,
    dest_community_id: String,
    dest_channel_id: String,
    state: State<'_, SharedState>,
) -> Result<SendChannelMessageResponse, String> {
    let pool = state.db.current()?;
    crate::services::messaging_runtime::forward_channel_message_inner(
        state.inner(),
        &pool,
        source_community_id,
        source_channel_id,
        source_message_id,
        dest_community_id,
        dest_channel_id,
    )
    .await
}

/// Post a message in a community channel
/// (`CommunityRequest::ChannelMessageSend`).
#[tauri::command]
pub async fn send_channel_message(
    channel_id: String,
    body: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::ChannelMessageSend {
        channel: channel_id,
        body,
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
