use rekindle_ipc::protocol::{ChannelMessageRequest, ChannelRequest, MemberRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

use rekindle_types::dto::community::CommunityBanView;

host_command! {
    state;
    /// Remove a member from a community without barring return.
    pub async fn remove_community_member(
        community_id: String,
        pseudonym_key: String,
        reason: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::Kick {
            community: community_id,
            target_pseudonym: pseudonym_key,
            reason,
        }))
    }
}

host_command! {
    state;
    pub async fn timeout_member(
        community_id: String,
        pseudonym_key: String,
        duration_seconds: u64,
        reason: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::Timeout {
            community: community_id,
            target_pseudonym: pseudonym_key,
            duration_seconds,
            reason,
        }))
    }
}

host_command! {
    state;
    pub async fn remove_timeout(
        community_id: String,
        pseudonym_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::TimeoutRemove {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }))
    }
}

host_command! {
    state;
    pub async fn set_channel_overwrite(
        community_id: String,
        channel_id: String,
        target_type: String,
        target_id: String,
        allow: u64,
        deny: u64,
        state: State<'_, SharedState>,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelOverwriteSet {
                community: community_id,
                channel_id,
                target_type,
                target_id,
                allow,
                deny,
            },
        ))
    }
}

host_command! {
    state;
    pub async fn delete_channel_overwrite(
        community_id: String,
        channel_id: String,
        target_type: String,
        target_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelOverwriteDelete {
                community: community_id,
                channel_id,
                target_type,
                target_id,
            },
        ))
    }
}

host_command! {
    state;
    pub async fn set_slowmode(
        community_id: String,
        channel_id: String,
        seconds: u32,
        state: State<'_, SharedState>,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelSetSlowmode {
                community: community_id,
                channel_id,
                seconds,
            },
        ))
    }
}

host_command! {
    state;
    pub async fn ban_member(
        community_id: String,
        pseudonym_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::Ban {
            community: community_id,
            target_pseudonym: pseudonym_key,
            reason: None,
        }))
    }
}

host_command! {
    state;
    pub async fn unban_member(
        community_id: String,
        pseudonym_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::Unban {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }))
    }
}

host_command! {
    state;
    /// Delete a member's channel message as a moderator
    /// (`CommunityRequest::ChannelMessageAdminDelete`, `MANAGE_MESSAGES`).
    pub async fn admin_delete_channel_message(
        community_id: String,
        channel_id: String,
        message_id: String,
        reason: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelMessageAdminDelete {
                community: community_id,
                channel: channel_id,
                message_id,
                reason,
            },
        ))
    }
}

host_command! {
    state;
    /// Delete up to 100 channel messages as a moderator
    /// (`CommunityRequest::ChannelMessageBulkDelete`): how many were deleted.
    pub async fn bulk_delete_channel_messages(
        community_id: String,
        channel_id: String,
        message_ids: Vec<String>,
        reason: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<u32>, CommandError> {
        IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelMessageBulkDelete {
                community: community_id,
                channel: channel_id,
                message_ids,
                reason,
            },
        ))
    }
}

host_command! {
    state;
    /// The community's bans, from the merged governance state.
    pub async fn get_ban_list(
        community_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<CommunityBanView>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::BanList {
            community: community_id,
        }))
    }
}
