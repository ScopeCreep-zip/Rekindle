//! A community's invites (plan E3.1 D1): each command is one Host request;
//! the Host validates it, reviews its intent and mints, lists or revokes.

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::{CommunityInviteCreated, CommunityInviteView};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Mint an invite (`IpcRequest::Community(CommunityRequest::InviteCreate)`);
/// no `max_uses` is single-use, as the CLI's `invite create` (readers refuse
/// one outside `1..=100`, plan E3.2 (d)).
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
            max_uses: invite_max_uses(max_uses),
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

/// The redemptions a GUI invite allows: the form's empty field is one.
fn invite_max_uses(asked: Option<u32>) -> u32 {
    asked.unwrap_or(1)
}

#[cfg(test)]
mod tests {
    /// An empty "max uses" field mints a single-use invite, never 0 (which
    /// every reader refuses).
    #[test]
    fn an_invite_without_max_uses_is_single_use() {
        assert_eq!(super::invite_max_uses(None), 1);
        assert_eq!(super::invite_max_uses(Some(7)), 7);
    }
}
