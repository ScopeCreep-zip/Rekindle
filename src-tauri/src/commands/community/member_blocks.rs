//! A community block by pseudonym (plan E3.1 F3, D26), answered by the Host.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn block_community_member(
    state: State<'_, SharedState>,
    community_id: String,
    pseudonym_key: String,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::MemberBlock {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }),
    )
    .await
}

#[tauri::command]
pub async fn unblock_community_member(
    state: State<'_, SharedState>,
    community_id: String,
    pseudonym_key: String,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::MemberUnblock {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }),
    )
    .await
}

#[tauri::command]
pub async fn list_community_blocks(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<Vec<String>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::MemberBlockList {
            community: community_id,
        }),
    )
    .await
}
