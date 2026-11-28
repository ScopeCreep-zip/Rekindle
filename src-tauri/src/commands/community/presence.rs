use rekindle_types::dto::community_profile::{CommunityProfilePatch, ProfileField};
use tauri::State;

use crate::state::SharedState;

use rekindle_ipc::protocol::{ChannelMessageRequest, LifecycleRequest, SettingsRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::CommunityMemberView;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::presence::{PresencePolicyField, PresenceSharingPolicy};

/// The composer of a channel changed: `typing` is whether it holds text
/// (`ChannelMessageRequest::ChannelTyping`; the Host decides what is sent).
#[tauri::command]
pub async fn send_channel_typing(
    community_id: String,
    channel_id: String,
    typing: bool,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Messages(
        ChannelMessageRequest::ChannelTyping {
            community: community_id,
            channel: channel_id,
            typing,
        },
    ));
    crate::host_call::call(&state, request).await
}

/// The channel we are in, or none (`CommunityRequest::ChannelSetActive`).
#[tauri::command]
pub async fn set_active_channel(
    community_id: String,
    channel_id: Option<String>,
    kind: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Messages(
        ChannelMessageRequest::ChannelSetActive {
            community: community_id,
            channel: channel_id,
            kind,
        },
    ));
    crate::host_call::call(&state, request).await
}

/// Our presence sharing consent in a community, kept by the Host
/// (`SettingsRequest::CommunitySetPresencePolicy`): the game our card
/// shares follows it within `CARD_GAME_SETTLE` (plan E3.4 S6).
#[tauri::command]
pub async fn set_presence_policy(
    community_id: String,
    policy: rekindle_types::presence::PresenceSharingPolicy,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunitySetPresencePolicy {
            community: community_id,
            policy,
        },
    ));
    crate::host_call::call(&state, request).await
}

/// Change one consent field in the Host's current policy.
#[tauri::command]
pub async fn set_presence_policy_field(
    community_id: String,
    field: PresencePolicyField,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunitySetPresencePolicyField {
            community: community_id,
            field,
        },
    ));
    crate::host_call::call(&state, request).await
}

/// Read the Host's policy, retaining the settings panel's existing payload.
#[tauri::command]
pub async fn get_presence_policy(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<PresenceSharingPolicy, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunityGetPresencePolicy {
            community: community_id,
        },
    ));
    crate::host_call::call(&state, request)
        .await
        .map(|done| done.value)
}

/// A community's member list (`IpcRequest::Community(CommunityRequest::CommunityMembers)`).
#[tauri::command]
pub async fn get_community_members(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<CommunityMemberView>>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityMembers {
            anchor: community_id,
        },
    ));
    crate::host_call::call(&state, request).await
}

#[tauri::command]
pub async fn update_community_profile(
    community_id: String,
    patch: CommunityProfilePatch,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunityUpdateProfile {
            community: community_id,
            patch,
        },
    ));
    crate::host_call::call(&state, request).await
}

/// The name on our per-community card (plan E3.4 S6).
#[tauri::command]
pub async fn set_community_display_name(
    community_id: String,
    display_name: ProfileField<String>,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunitySetDisplayName {
            community: community_id,
            display_name,
        },
    ));
    crate::host_call::call(&state, request).await
}
