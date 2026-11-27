//! Channel delete and rename (plan E3.1 E1): Host calls.

use rekindle_ipc::protocol::ChannelRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    /// Delete a channel; the Host holds it for review when it has history or readers (`ContainerNotEmpty`).
    pub async fn delete_channel(
        community_id: String,
        channel_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelDelete {
            community: community_id,
            channel_id,
        }))
    }
}

host_command! {
    state;
    pub async fn rename_channel(
        community_id: String,
        channel_id: String,
        new_name: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelRename {
            community: community_id,
            channel_id,
            name: new_name,
        }))
    }
}
