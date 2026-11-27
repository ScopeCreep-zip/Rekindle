use rekindle_ipc::protocol::AuditRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::CommunityAuditEntry;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    /// A page of a community's audit log, newest first (plan E3.1 F4).
    pub async fn get_audit_log(
        state: State<'_, SharedState>,
        community_id: String,
        before_timestamp: Option<u64>,
        limit: u32,
    ) -> Result<Done<Vec<CommunityAuditEntry>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Audit(AuditRequest::AuditLog {
            community: community_id,
            before_timestamp,
            limit,
        }))
    }
}
