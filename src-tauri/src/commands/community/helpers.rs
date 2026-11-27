use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;

use crate::state::SharedState;
use crate::state_helpers::host_governance;

pub(crate) fn require_permission(
    state: &SharedState,
    community_id: &str,
    required: u64,
) -> Result<(), String> {
    host_governance(state)?
        .require_permission(community_id, required)
        .map_err(|e| e.to_string())
}
