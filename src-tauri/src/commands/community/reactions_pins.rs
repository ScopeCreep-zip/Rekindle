use tauri::State;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::PinnedMessageView;
use rekindle_types::dto::{CommandError, Done};

use crate::host_call;
use crate::state::SharedState;
use rekindle_types::permissions;

use super::helpers::require_permission;

/// Add a reaction to a community channel message.
#[tauri::command]
pub async fn add_reaction(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<(), String> {
    let pool = state.db.current()?;
    let _ = pool;
    require_permission(state.inner(), &community_id, permissions::SEND_MESSAGES)?;
    crate::services::community::persist_reaction(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &emoji,
        true,
    )
    .await
}

/// Remove a reaction from a community channel message.
#[tauri::command]
pub async fn remove_reaction(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<(), String> {
    let pool = state.db.current()?;
    let _ = pool;
    require_permission(state.inner(), &community_id, permissions::SEND_MESSAGES)?;
    crate::services::community::persist_reaction(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &emoji,
        false,
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
        IpcRequest::Community(CommunityRequest::MessagePin {
            community: community_id,
            channel_id,
            message_id,
        }),
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
        IpcRequest::Community(CommunityRequest::MessageUnpin {
            community: community_id,
            channel_id,
            message_id,
        }),
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
        IpcRequest::Community(CommunityRequest::ChannelPins {
            community: community_id,
            channel_id,
        }),
    )
    .await
}
