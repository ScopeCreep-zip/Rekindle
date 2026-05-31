use tauri::State;

use crate::db::DbPool;
use crate::services::community_audit_runtime::get_audit_log_inner;
use crate::state::SharedState;

use super::types::AuditLogEntryInfoDto;

/// Get paginated audit log entries for a community.
#[tauri::command]
pub async fn get_audit_log(
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
    community_id: String,
    before_timestamp: Option<u64>,
    limit: u32,
) -> Result<Vec<AuditLogEntryInfoDto>, String> {
    get_audit_log_inner(state.inner(), community_id, before_timestamp, limit).await
}
