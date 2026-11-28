//! Channel and category administration (plan E3.1 E1): Host calls.

use rekindle_ipc::protocol::ChannelRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Create a channel; `parent_voice_channel_id` makes it the text-in-voice companion of that voice channel (architecture §10.8).
#[tauri::command]
pub async fn create_channel(
    community_id: String,
    name: String,
    channel_type: String,
    category_id: Option<String>,
    parent_voice_channel_id: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelCreate {
            community: community_id,
            name,
            kind: channel_type,
            category: category_id,
            topic: None,
            slowmode_seconds: 0,
            parent_voice_channel: parent_voice_channel_id,
        })),
    )
    .await
}

#[tauri::command]
pub async fn create_category(
    community_id: String,
    name: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::CategoryCreate {
            community: community_id,
            name,
        })),
    )
    .await
}

#[tauri::command]
pub async fn delete_category(
    community_id: String,
    category_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::CategoryDelete {
            community: community_id,
            category_id,
        })),
    )
    .await
}

#[tauri::command]
pub async fn rename_category(
    community_id: String,
    category_id: String,
    new_name: String,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::CategoryRename {
            community: community_id,
            category_id,
            name: new_name,
        })),
    )
    .await
}

#[tauri::command]
pub async fn move_channel(
    community_id: String,
    channel_id: String,
    category_id: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelMove {
            community: community_id,
            channel_id,
            category: category_id,
        })),
    )
    .await
}

#[tauri::command]
pub async fn reorder_categories(
    community_id: String,
    category_ids: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::CategoryReorder {
                community: community_id,
                category_ids,
            },
        )),
    )
    .await
}

#[tauri::command]
pub async fn set_channel_topic(
    community_id: String,
    channel_id: String,
    topic: String,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelSetTopic {
                community: community_id,
                channel_id,
                topic,
            },
        )),
    )
    .await
}

#[tauri::command]
pub async fn set_channel_forum_tags(
    community_id: String,
    channel_id: String,
    forum_tags: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelSetForumTags {
                community: community_id,
                channel_id,
                forum_tags,
            },
        )),
    )
    .await
}

#[tauri::command]
pub async fn reorder_channels(
    community_id: String,
    channel_ids: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelReorder {
            community: community_id,
            channel_ids,
        })),
    )
    .await
}
