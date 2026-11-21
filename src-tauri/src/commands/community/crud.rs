use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;

use crate::state::SharedState;
use rekindle_types::dto::community::{
    CoCreationSnapshot, CommunityCreated, CommunityJoined, CommunityView,
};

/// The joined communities (`IpcRequest::Community(CommunityRequest::CommunityList)`).
#[tauri::command]
pub async fn get_communities(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<rekindle_types::display::CommunityOverview>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::CommunityList)),
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
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        )),
    )
    .await
}

/// The held co-creator invites and our pending creations' co-creator
/// states (`LifecycleRequest::CoCreationSnapshot`, plan E3.2 S7s.3g), read
/// once the window's event stream is live.
#[tauri::command]
pub async fn get_co_creation(
    state: State<'_, SharedState>,
) -> Result<Done<CoCreationSnapshot>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CoCreationSnapshot,
        )),
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
    co_creators: Option<Vec<String>>,
    state: State<'_, SharedState>,
) -> Result<Done<CommunityCreated>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityCreate {
            name,
            description: String::new(),
            approval_required: approval_required.unwrap_or(false),
            co_creators: co_creators.unwrap_or_default(),
        },
    ));
    host_call::call(&state, request).await
}

/// Stop waiting on a co-creator of our pending creation that has not
/// answered (`LifecycleRequest::CreatorRemove`, plan E3.2 S7s.3).
#[tauri::command]
pub async fn remove_co_creator(
    community_id: String,
    identity: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CreatorRemove {
            anchor: community_id,
            identity,
        },
    ));
    host_call::call(&state, request).await
}

/// Publish a successor of a community we created
/// (`LifecycleRequest::CommunityPublishSuccessor`, plan E3.2 S7s.4).
#[tauri::command]
pub async fn publish_community_successor(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityPublishSuccessor {
            anchor: community_id,
        },
    ));
    host_call::call(&state, request).await
}

/// Follow an offered successor with an invite to it
/// (`LifecycleRequest::CommunityFollowSuccessor`, plan E3.2 S7s.4).
#[tauri::command]
pub async fn follow_community_successor(
    community_id: String,
    new_anchor: String,
    invite: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityFollowSuccessor {
            anchor: community_id,
            new_anchor,
            invite,
        },
    ));
    host_call::call(&state, request).await
}

/// The successors offered for a community this session
/// (`LifecycleRequest::CommunitySuccessors`, lead decision (33)).
#[tauri::command]
pub async fn get_community_successors(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunitySuccessors {
            anchor: community_id,
        },
    ));
    host_call::call(&state, request).await
}

/// Cancel our pending creation (`LifecycleRequest::CommunityCreateCancel`).
#[tauri::command]
pub async fn cancel_community_create(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityCreateCancel {
            anchor: community_id,
        },
    ));
    host_call::call(&state, request).await
}

/// Accept or decline a held co-creator invite
/// (`LifecycleRequest::CreatorInviteAnswer`).
#[tauri::command]
pub async fn answer_creator_invite(
    community_id: String,
    accept: bool,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CreatorInviteAnswer {
            anchor: community_id,
            accept,
        },
    ));
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
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityJoin { invite: invite_url },
    ));
    host_call::call(&state, request).await
}

/// Leave a community (`IpcRequest::Community(CommunityRequest::CommunityLeave)`), reviewed with its
/// leave escalations.
#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityLeave {
            anchor: community_id,
        },
    ));
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
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityUpdateInfo {
            anchor: community_id,
            name,
            description,
            icon_hash,
            banner_hash,
        },
    ));
    host_call::call(&state, request).await
}
