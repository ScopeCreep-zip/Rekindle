//! Read markers and unread counts (plan E3.1 G1, D25): Host calls.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::ChannelUnreadView;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn mark_channel_read(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    last_message_id: String,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::ChannelMarkRead {
        community: community_id,
        channel: channel_id,
        up_to: last_message_id,
    });
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn get_unread_counts(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<Vec<ChannelUnreadView>>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::UnreadCounts {
        community: community_id,
    });
    host_call::call(&state, request).await
}
