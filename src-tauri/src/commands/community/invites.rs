//! A community's invites (plan E3.1 D1): each command is one Host request;
//! the Host validates it, reviews its intent and mints, lists or revokes.

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::{CommunityInviteCreated, CommunityInviteView};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    /// Mint an invite (`IpcRequest::Community(CommunityRequest::InviteCreate)`).
    /// `max_uses` is the form's explicit choice (IJ1: never an absent field
    /// read as a default); the Host refuses one outside `1..=100`, which every
    /// reader refuses (plan E3.2 (d), `invite_quota::check_max_uses_cap`).
    pub async fn create_community_invite(
        community_id: String,
        max_uses: u32,
        expires_in_seconds: Option<u64>,
        state: State<'_, SharedState>,
    ) -> Result<Done<CommunityInviteCreated>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteCreate {
                community: community_id,
                max_uses,
                expires_seconds: expires_in_seconds,
            },
        ))
    }
}

host_command! {
    state;
    /// Revoke the invite `invite_id` (`IpcRequest::Community(CommunityRequest::InviteRevoke)`).
    pub async fn revoke_community_invite(
        community_id: String,
        invite_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteRevoke {
                community: community_id,
                invite_code: invite_id,
            },
        ))
    }
}

host_command! {
    state;
    /// The community's live invites (`IpcRequest::Community(CommunityRequest::InviteList)`).
    pub async fn list_community_invites(
        community_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<CommunityInviteView>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::InviteList {
            community: community_id,
        }))
    }
}
