//! A community's policy (architecture §10.7, §20.6; plan E3.1 D4): Host calls.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::CommunityPolicyView;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn get_community_policy(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<CommunityPolicyView>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityPolicyGet {
        community: community_id,
    });
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn set_community_policy(
    state: State<'_, SharedState>,
    community_id: String,
    policy_text: Option<String>,
    max_joins_per_interval: u32,
    join_interval_seconds: u32,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityPolicySet {
        community: community_id,
        policy_text,
        max_joins_per_interval,
        join_interval_seconds,
    });
    host_call::call(&state, request).await
}
