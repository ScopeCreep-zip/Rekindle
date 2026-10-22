use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::CommunityAuditEntry;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// A page of a community's audit log, newest first (plan E3.1 F4).
#[tauri::command]
pub async fn get_audit_log(
    state: State<'_, SharedState>,
    community_id: String,
    before_timestamp: Option<u64>,
    limit: u32,
) -> Result<Done<Vec<CommunityAuditEntry>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::AuditLog {
            community: community_id,
            before_timestamp,
            limit,
        }),
    )
    .await
}
