//! Read markers and unread counts (plan E3.1 G1, D25) and the members on
//! screen (plan E3.4 Q1): Host calls.

use rekindle_ipc::protocol::UnreadRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::ChannelUnreadView;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    pub async fn mark_channel_read(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        last_message_id: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Unread(UnreadRequest::ChannelMarkRead {
            community: community_id,
            channel: channel_id,
            up_to: last_message_id,
        }))
    }
}

host_command! {
    state;
    /// The members the member list shows (plan E3.4 Q1, at most 50): the Host
    /// keeps their beats live beside the ring's.
    pub async fn set_members_on_screen(
        state: State<'_, SharedState>,
        community_id: String,
        members: Vec<String>,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Unread(UnreadRequest::MembersOnScreen {
            community: community_id,
            members,
        }))
    }
}

host_command! {
    state;
    pub async fn get_unread_counts(
        state: State<'_, SharedState>,
        community_id: String,
    ) -> Result<Done<Vec<ChannelUnreadView>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Unread(UnreadRequest::UnreadCounts {
            community: community_id,
        }))
    }
}
