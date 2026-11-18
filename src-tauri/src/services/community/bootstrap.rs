//! The inviter's side of the join's bootstrap exchange (plan E3.2):
//! `rekindle_governance_runtime::build_bootstrap_response_v2` over the
//! Host's `HostGovernance`, the same function the daemon runs. `Ok(None)`:
//! the call is left unanswered (a partial community, S7c.8 (1)).

use std::sync::Arc;

use crate::state::AppState;

pub async fn build_bootstrap_response_v2(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    anchor: &str,
    joiner: &rekindle_governance_runtime::BootstrapJoiner,
) -> Result<Option<Vec<u8>>, String> {
    let adapter = crate::state_helpers::host_governance(state)?;
    let response =
        match rekindle_governance_runtime::build_bootstrap_response_v2(&adapter, anchor, joiner)
            .await
        {
            Ok(response) => response,
            // Partial (plan E3.2 S7c.8 (1)): no answer, never a refusal.
            Err(rekindle_governance_runtime::GovernanceRuntimeError::PartialState(_)) => {
                return Ok(None)
            }
            Err(e) => return Err(e.to_string()),
        };
    // The joiner's `MemberJoined` merged: the community key follows by
    // `MekTransfer` (D16), never in the response (D11).
    let pool = state.db.current().map_err(|e| e.to_string())?;
    let deps =
        crate::services::mek_adapter::MekAdapter::new(Arc::clone(state), app_handle.clone(), pool);
    adapter
        .deliver_key_to_joiner(deps.as_ref(), anchor, joiner)
        .await;
    Ok(Some(response))
}
