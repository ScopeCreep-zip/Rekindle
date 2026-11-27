use tauri::State;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_ipc::protocol::{PinRequest, RichRequest};
use rekindle_types::dto::community::PinnedMessageView;
use rekindle_types::dto::{CommandError, Done};

use crate::state::SharedState;

host_command! {
    state;
    /// React to a channel message (`CommunityRequest::ReactionAdd`).
    pub async fn add_reaction(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        message_id: String,
        emoji: String,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ReactionAdd {
            community: community_id,
            channel_id,
            message_id,
            emoji,
        }))
    }
}

host_command! {
    state;
    /// Take our reaction off a channel message (`CommunityRequest::ReactionRemove`).
    pub async fn remove_reaction(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        message_id: String,
        emoji: String,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ReactionRemove {
            community: community_id,
            channel_id,
            message_id,
            emoji,
        }))
    }
}

host_command! {
    state;
    /// Pin a message in a community channel (the Host's, plan E3.1 E3b).
    pub async fn pin_message(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        message_id: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Pins(PinRequest::MessagePin {
            community: community_id,
            channel_id,
            message_id,
        }))
    }
}

host_command! {
    state;
    /// Unpin a message from a community channel.
    pub async fn unpin_message(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        message_id: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Pins(PinRequest::MessageUnpin {
            community: community_id,
            channel_id,
            message_id,
        }))
    }
}

host_command! {
    state;
    /// A community channel's pinned messages.
    pub async fn get_channel_pins(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
    ) -> Result<Done<Vec<PinnedMessageView>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Pins(PinRequest::ChannelPins {
            community: community_id,
            channel_id,
        }))
    }
}
