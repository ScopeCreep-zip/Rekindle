use rekindle_ipc::protocol::{ChannelMessageRequest, ChannelRequest, MemberRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;

use crate::state::SharedState;

use rekindle_types::dto::community::CommunityBanView;

/// Remove a member from a community without barring return.
#[tauri::command]
pub async fn remove_community_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::Kick {
            community: community_id,
            target_pseudonym: pseudonym_key,
        })),
    )
    .await
}

#[tauri::command]
pub async fn timeout_member(
    community_id: String,
    pseudonym_key: String,
    duration_seconds: u64,
    reason: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::Timeout {
            community: community_id,
            target_pseudonym: pseudonym_key,
            duration_seconds,
            reason,
        })),
    )
    .await
}

#[tauri::command]
pub async fn remove_timeout(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::TimeoutRemove {
            community: community_id,
            target_pseudonym: pseudonym_key,
        })),
    )
    .await
}

#[tauri::command]
pub async fn set_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    allow: u64,
    deny: u64,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelOverwriteSet {
                community: community_id,
                channel_id,
                target_type,
                target_id,
                allow,
                deny,
            },
        )),
    )
    .await
}

#[tauri::command]
pub async fn delete_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelOverwriteDelete {
                community: community_id,
                channel_id,
                target_type,
                target_id,
            },
        )),
    )
    .await
}

#[tauri::command]
pub async fn set_slowmode(
    community_id: String,
    channel_id: String,
    seconds: u32,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelSetSlowmode {
                community: community_id,
                channel_id,
                seconds,
            },
        )),
    )
    .await
}

#[tauri::command]
pub async fn ban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::Ban {
            community: community_id,
            target_pseudonym: pseudonym_key,
            reason: None,
        })),
    )
    .await
}

#[tauri::command]
pub async fn unban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::Unban {
            community: community_id,
            target_pseudonym: pseudonym_key,
        })),
    )
    .await
}

/// Delete a member's channel message as a moderator
/// (`CommunityRequest::ChannelMessageAdminDelete`, `MANAGE_MESSAGES`).
#[tauri::command]
pub async fn admin_delete_channel_message(
    community_id: String,
    channel_id: String,
    message_id: String,
    reason: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelMessageAdminDelete {
                community: community_id,
                channel: channel_id,
                message_id,
                reason,
            },
        )),
    )
    .await
}

/// Delete up to 100 channel messages as a moderator
/// (`CommunityRequest::ChannelMessageBulkDelete`): how many were deleted.
#[tauri::command]
pub async fn bulk_delete_channel_messages(
    community_id: String,
    channel_id: String,
    message_ids: Vec<String>,
    reason: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<u32>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelMessageBulkDelete {
                community: community_id,
                channel: channel_id,
                message_ids,
                reason,
            },
        )),
    )
    .await
}

/// The community's bans, from the merged governance state.
#[tauri::command]
pub async fn get_ban_list(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<CommunityBanView>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::BanList {
            community: community_id,
        })),
    )
    .await
}
