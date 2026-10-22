//! Threads (plan E3.1 E3), answered by the Host.

use rekindle_ipc::protocol::RichRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;
use rekindle_types::dto::channel_message::ChannelMessageView;
use rekindle_types::thread::ThreadInfo;

/// Open a thread off a channel message (`CommunityRequest::ThreadCreate`).
#[tauri::command]
pub async fn create_thread(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    name: String,
    starter_message_id: String,
    forum_tag: Option<String>,
    auto_archive_seconds: Option<u64>,
) -> Result<Done<String>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadCreate {
            community: community_id,
            channel_id,
            name,
            starter_message_id,
            forum_tag,
            auto_archive_seconds,
        })),
    )
    .await
}

/// A channel's threads (`CommunityRequest::ThreadList`).
#[tauri::command]
pub async fn get_channel_threads(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
) -> Result<Done<Vec<ThreadInfo>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadList {
            community: community_id,
            channel_id,
        })),
    )
    .await
}

/// A channel's open threads (`CommunityRequest::ThreadListActive`).
#[tauri::command]
pub async fn get_active_threads(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
) -> Result<Done<Vec<ThreadInfo>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadListActive {
            community: community_id,
            channel_id,
        })),
    )
    .await
}

/// Post to a thread (`CommunityRequest::ThreadSend`).
#[tauri::command]
pub async fn send_thread_message(
    state: State<'_, SharedState>,
    community_id: String,
    thread_id: String,
    body: String,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadSend {
            community: community_id,
            thread_id,
            body,
        })),
    )
    .await
}

/// A page of a thread's messages (`CommunityRequest::ThreadMessages`).
#[tauri::command]
pub async fn get_thread_messages(
    state: State<'_, SharedState>,
    community_id: String,
    thread_id: String,
    limit: u32,
    before_timestamp: Option<u64>,
) -> Result<Done<Vec<ChannelMessageView>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadMessages {
            community: community_id,
            thread_id,
            limit,
            before_timestamp,
        })),
    )
    .await
}

/// Archive a thread (`CommunityRequest::ThreadArchive`).
#[tauri::command]
pub async fn archive_thread(
    state: State<'_, SharedState>,
    community_id: String,
    thread_id: String,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadArchive {
            community: community_id,
            thread_id,
        })),
    )
    .await
}

/// Reopen a thread (`CommunityRequest::ThreadUnarchive`).
#[tauri::command]
pub async fn unarchive_thread(
    state: State<'_, SharedState>,
    community_id: String,
    thread_id: String,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadUnarchive {
            community: community_id,
            thread_id,
        })),
    )
    .await
}
