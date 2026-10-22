use tauri::State;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_ipc::protocol::{PinRequest, RichRequest};
use rekindle_types::dto::community::PinnedMessageView;
use rekindle_types::dto::{CommandError, Done};

use crate::host_call;
use crate::state::SharedState;

/// React to a channel message (`CommunityRequest::ReactionAdd`).
#[tauri::command]
pub async fn add_reaction(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ReactionAdd {
            community: community_id,
            channel_id,
            message_id,
            emoji,
        })),
    )
    .await
}

/// Take our reaction off a channel message (`CommunityRequest::ReactionRemove`).
#[tauri::command]
pub async fn remove_reaction(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ReactionRemove {
            community: community_id,
            channel_id,
            message_id,
            emoji,
        })),
    )
    .await
}

/// Pin a message in a community channel (the Host's, plan E3.1 E3b).
#[tauri::command]
pub async fn pin_message(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    message_id: String,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Pins(PinRequest::MessagePin {
            community: community_id,
            channel_id,
            message_id,
        })),
    )
    .await
}

/// Unpin a message from a community channel.
#[tauri::command]
pub async fn unpin_message(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    message_id: String,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Pins(PinRequest::MessageUnpin {
            community: community_id,
            channel_id,
            message_id,
        })),
    )
    .await
}

/// A community channel's pinned messages.
#[tauri::command]
pub async fn get_channel_pins(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
) -> Result<Done<Vec<PinnedMessageView>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Pins(PinRequest::ChannelPins {
            community: community_id,
            channel_id,
        })),
    )
    .await
}
