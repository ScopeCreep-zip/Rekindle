use tauri::State;

use crate::commands::chat::Message;
use crate::db::DbPool;
use crate::services::messaging_runtime::SendChannelMessageResponse;
use crate::state::SharedState;

/// Forward a previously-cached channel message to another channel.
///
/// `source_*` and `dest_*` may identify the same community or different
/// communities. The source must already be cached locally (no DHT refetch).
/// Re-encrypts content with the destination MEK and writes a
/// `ChannelEntry::Forward` entry plus a gossip notification.
#[tauri::command]
#[allow(clippy::too_many_arguments, reason = "Tauri command surface — matches forward_channel_message args")]
pub async fn forward_channel_message(
    source_community_id: String,
    source_channel_id: String,
    source_message_id: String,
    dest_community_id: String,
    dest_channel_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<SendChannelMessageResponse, String> {
    crate::services::messaging_runtime::forward_channel_message_inner(
        state.inner(),
        pool.inner(),
        &app,
        source_community_id,
        source_channel_id,
        source_message_id,
        dest_community_id,
        dest_channel_id,
    )
    .await
}

/// Send a message in a community channel.
#[tauri::command]
pub async fn send_channel_message(
    channel_id: String,
    body: String,
    reply_to_id: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<SendChannelMessageResponse, String> {
    let _ = reply_to_id;
    crate::services::messaging_runtime::send_channel_message_inner(
        state.inner(),
        pool.inner(),
        &app,
        channel_id,
        body,
    )
    .await
}

#[tauri::command]
pub async fn edit_channel_message(
    channel_id: String,
    message_id: String,
    new_body: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::services::messaging_runtime::edit_channel_message_inner(
        state.inner(),
        channel_id,
        message_id,
        &new_body,
    )
}

#[tauri::command]
pub async fn delete_channel_message(
    channel_id: String,
    message_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    crate::services::messaging_runtime::delete_channel_message_inner(
        state.inner(),
        channel_id,
        message_id,
    )
}

#[tauri::command]
pub async fn get_channel_messages(
    channel_id: String,
    limit: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    crate::services::messaging_runtime::get_channel_messages_inner(
        state.inner().clone(),
        pool.inner().clone(),
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
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    crate::services::messaging_runtime::get_older_channel_messages_inner(
        state.inner().clone(),
        pool.inner().clone(),
        community_id,
        channel_id,
        before_timestamp,
        limit,
    )
    .await
}
