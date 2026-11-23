//! The inviter's side of the join's bootstrap exchange (plan E3.2):
//! `rekindle_governance_runtime::answer_bootstrap_request` over the Host's
//! `HostGovernance`, the same function the daemon runs. A partial community
//! answers `bootstrapPartial` (S7c.8 (i)), never a refusal: the joiner asks
//! another member.

use std::sync::Arc;

use rekindle_governance_runtime::BootstrapAnswer;

use crate::state::AppState;

pub async fn answer_bootstrap_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    anchor: &str,
    joiner: &rekindle_governance_runtime::BootstrapJoiner,
) -> Result<Vec<u8>, String> {
    let adapter = crate::state_helpers::host_governance(state)?;
    match rekindle_governance_runtime::answer_bootstrap_request(&adapter, anchor, joiner)
        .await
        .map_err(|e| e.to_string())?
    {
        BootstrapAnswer::Frontier(response) => {
            // The joiner's `MemberJoined` merged: the community key follows
            // by `MekTransfer` (D16), never in the response (D11).
            let pool = state.db.current().map_err(|e| e.to_string())?;
            let deps = crate::services::mek_adapter::MekAdapter::new(
                Arc::clone(state),
                app_handle.clone(),
                pool,
            );
            adapter
                .deliver_key_to_joiner(deps.as_ref(), anchor, joiner)
                .await;
            Ok(response)
        }
        // Partial, the typed refusal (IJ2), or retry later (JN1: a dep of
        // the join is not fetchable yet): the runtime's answer, sent as is
        // so the joiner learns why, never a bare NAK.
        BootstrapAnswer::Partial(response)
        | BootstrapAnswer::Refused(response)
        | BootstrapAnswer::RetryLater(response) => Ok(response),
    }
}
