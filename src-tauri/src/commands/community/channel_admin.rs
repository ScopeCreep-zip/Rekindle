//! Channel delete and rename (plan E3.1 E1): Host calls.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Delete a channel; the Host holds it for review when it has history or readers (`ContainerNotEmpty`).
#[tauri::command]
pub async fn delete_channel(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::ChannelDelete {
            community: community_id,
            channel_id,
        },
    )
    .await
}

#[tauri::command]
pub async fn rename_channel(
    community_id: String,
    channel_id: String,
    new_name: String,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::ChannelRename {
            community: community_id,
            channel_id,
            name: new_name,
        },
    )
    .await
}
