use tauri::State;

use crate::services::community_presence_runtime::{
    get_presence_policy_inner, set_presence_policy_inner, update_community_presence_inner,
};
use crate::state::SharedState;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::CommunityMemberView;
use rekindle_types::dto::{CommandError, Done};

/// Tell the community we are typing (`CommunityRequest::ChannelTyping`).
#[tauri::command]
pub async fn send_channel_typing(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::ChannelTyping {
        community: community_id,
        channel: channel_id,
    });
    crate::host_call::call(&state, request).await
}

#[tauri::command]
pub async fn update_community_presence(
    community_id: String,
    status: String,
    game_name: Option<String>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    update_community_presence_inner(
        state.inner(),
        community_id,
        status,
        game_name,
        game_id,
        elapsed_seconds,
        server_address,
    )
    .await
}

/// The channel we are in, or none (`CommunityRequest::ChannelSetActive`).
#[tauri::command]
pub async fn set_active_channel(
    community_id: String,
    channel_id: Option<String>,
    kind: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::ChannelSetActive {
        community: community_id,
        channel: channel_id,
        kind,
    });
    crate::host_call::call(&state, request).await
}

#[tauri::command]
pub async fn set_presence_policy(
    community_id: String,
    policy: rekindle_types::presence::PresenceSharingPolicy,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    set_presence_policy_inner(state.inner(), &pool, community_id, policy).await
}

#[tauri::command]
pub async fn get_presence_policy(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<rekindle_types::presence::PresenceSharingPolicy, String> {
    Ok(get_presence_policy_inner(state.inner(), &community_id))
}

/// A community's member list (`IpcRequest::Community(CommunityRequest::CommunityMembers)`).
#[tauri::command]
pub async fn get_community_members(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<CommunityMemberView>>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityMembers {
        governance_key: community_id,
    });
    crate::host_call::call(&state, request).await
}

#[tauri::command]
pub async fn update_community_profile(
    community_id: String,
    bio: Option<String>,
    pronouns: Option<String>,
    theme_color: Option<u32>,
    badges: Vec<String>,
    avatar_ref: Option<String>,
    banner_ref: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityUpdateProfile {
        community: community_id,
        bio,
        pronouns,
        theme_color,
        badges,
        avatar_ref,
        banner_ref,
    });
    crate::host_call::call(&state, request).await
}
