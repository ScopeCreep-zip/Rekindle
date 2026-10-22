//! A community's invites (plan E3.1 D1): each command is one Host request;
//! the Host validates it, reviews its intent and mints, lists or revokes.

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::{CommunityInviteCreated, CommunityInviteView};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Mint an invite (`IpcRequest::Community(CommunityRequest::InviteCreate)`); no `max_uses` is unlimited.
#[tauri::command]
pub async fn create_community_invite(
    community_id: String,
    max_uses: Option<u32>,
    expires_in_seconds: Option<u64>,
    state: State<'_, SharedState>,
) -> Result<Done<CommunityInviteCreated>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::InviteCreate {
            community: community_id,
            max_uses: max_uses.unwrap_or(0),
            expires_seconds: expires_in_seconds,
        },
    ));
    host_call::call(&state, request).await
}

/// Revoke the invite `invite_id` (`IpcRequest::Community(CommunityRequest::InviteRevoke)`).
#[tauri::command]
pub async fn revoke_community_invite(
    community_id: String,
    invite_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::InviteRevoke {
            community: community_id,
            invite_code: invite_id,
        },
    ));
    host_call::call(&state, request).await
}

/// The community's live invites (`IpcRequest::Community(CommunityRequest::InviteList)`).
#[tauri::command]
pub async fn list_community_invites(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<CommunityInviteView>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::InviteList {
            community: community_id,
        })),
    )
    .await
}
