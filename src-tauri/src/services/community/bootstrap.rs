//! The inviter's side of the join's bootstrap exchange (plan E3.2):
//! `rekindle_governance_runtime::build_bootstrap_response_v2` over the
//! Host's `HostGovernance`, the same function the daemon runs.

use std::sync::Arc;

use crate::state::AppState;

pub async fn build_bootstrap_response_v2(
    state: &Arc<AppState>,
    anchor: &str,
    joiner: &rekindle_governance_runtime::BootstrapJoiner,
) -> Result<Vec<u8>, String> {
    let adapter = crate::state_helpers::host_governance(state)?;
    rekindle_governance_runtime::build_bootstrap_response_v2(&adapter, anchor, joiner)
        .await
        .map_err(|e| e.to_string())
}
