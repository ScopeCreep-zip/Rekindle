use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;

use crate::state::SharedState;
use rekindle_types::dto::community::{CommunityCreated, CommunityJoined, CommunityView};

/// The joined communities (`IpcRequest::Community(CommunityRequest::CommunityList)`).
#[tauri::command]
pub async fn get_communities(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<rekindle_types::display::CommunityOverview>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::CommunityList),
    )
    .await
}

/// Every joined community as its window renders it
/// (`IpcRequest::Community(CommunityRequest::CommunityViews)`).
#[tauri::command]
pub async fn get_community_details(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<CommunityView>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::CommunityViews),
    )
    .await
}

/// Create a community (`IpcRequest::Community(CommunityRequest::CommunityCreate)`).
///
/// `approval_required` picks the genesis admission mode and is
/// immutable afterwards: the CRDT honours `AdmissionPolicy` only as the
/// genesis entry, so a later one is ignored by every honest peer.
#[tauri::command]
pub async fn create_community(
    name: String,
    approval_required: Option<bool>,
    state: State<'_, SharedState>,
) -> Result<Done<CommunityCreated>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityCreate {
        name,
        description: String::new(),
        approval_required: approval_required.unwrap_or(false),
    });
    host_call::call(&state, request).await
}

/// Join a community from its invite link
/// (`IpcRequest::Community(CommunityRequest::CommunityJoin)`), reviewed with
/// the fingerprint of the invite's record key.
#[tauri::command]
pub async fn join_community(
    invite_url: String,
    state: State<'_, SharedState>,
) -> Result<Done<CommunityJoined>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityJoin { invite: invite_url });
    host_call::call(&state, request).await
}

/// Leave a community (`IpcRequest::Community(CommunityRequest::CommunityLeave)`), reviewed with its
/// leave escalations.
#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityLeave {
        governance_key: community_id,
    });
    host_call::call(&state, request).await
}

/// Edit the community's name, description, icon or banner; `None` keeps
/// the merged value (`IpcRequest::Community(CommunityRequest::CommunityUpdateInfo)`).
#[tauri::command]
pub async fn update_community_info(
    community_id: String,
    name: Option<String>,
    description: Option<String>,
    icon_hash: Option<String>,
    banner_hash: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityUpdateInfo {
        governance_key: community_id,
        name,
        description,
        icon_hash,
        banner_hash,
    });
    host_call::call(&state, request).await
}
