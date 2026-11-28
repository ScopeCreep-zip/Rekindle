//! The inviter's side of the join's bootstrap exchange (plan E3.2):
//! `rekindle_governance_runtime::answer_bootstrap_request` over the Host's
//! `HostGovernance`, the same function the daemon runs. A partial community
//! answers `bootstrapPartial` (S7c.8 (i)), never a refusal: the joiner asks
//! another member.

use std::sync::Arc;

use rekindle_governance_runtime::BootstrapAnswer;

use crate::state::AppState;

/// The desktop's wire answer to a joiner's `BootstrapRequest`: the
/// runtime's typed answer as is, and a bare NAK only when it could not
/// answer at all (not in the community, a malformed op, a store failure),
/// as the daemon's `dispatch_bare_request` does. A refusal and a retry
/// later reach the joiner as their own bytes, never as the NAK that
/// withdraws its join (plan E3.2 IJ2, JN1, BR1).
pub async fn answer_bootstrap_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    anchor: &str,
    joiner: &rekindle_governance_runtime::BootstrapJoiner,
) -> Vec<u8> {
    let answer = judge(state, anchor, joiner).await;
    let answer = match answer {
        Ok(BootstrapAnswer::Frontier(response)) => deliver_key(app_handle, state, anchor, joiner)
            .await
            .map(|()| BootstrapAnswer::Frontier(response)),
        other => other,
    };
    wire_reply(anchor, answer)
}

/// The runtime's answer over the Host's `HostGovernance`, the same
/// function the daemon runs.
async fn judge(
    state: &Arc<AppState>,
    anchor: &str,
    joiner: &rekindle_governance_runtime::BootstrapJoiner,
) -> Result<BootstrapAnswer, String> {
    let adapter = crate::state_helpers::host_governance(state)?;
    rekindle_governance_runtime::answer_bootstrap_request(&adapter, anchor, joiner)
        .await
        .map_err(|e| e.to_string())
}

/// The joiner's `MemberJoined` merged: the community key follows by
/// `MekTransfer` (D16), never in the response (D11).
async fn deliver_key(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    anchor: &str,
    joiner: &rekindle_governance_runtime::BootstrapJoiner,
) -> Result<(), String> {
    let adapter = crate::state_helpers::host_governance(state)?;
    let pool = state.db.current().map_err(|e| e.to_string())?;
    let deps =
        crate::services::mek_adapter::MekAdapter::new(Arc::clone(state), app_handle.clone(), pool);
    adapter
        .deliver_key_to_joiner(deps.as_ref(), anchor, joiner)
        .await;
    Ok(())
}

/// The bytes on the wire: every typed answer (frontier, partial, the
/// typed refusal, retry later) as is; a NAK only for no answer.
fn wire_reply(anchor: &str, answer: Result<BootstrapAnswer, String>) -> Vec<u8> {
    match answer {
        Ok(
            BootstrapAnswer::Frontier(response)
            | BootstrapAnswer::Partial(response)
            | BootstrapAnswer::Refused(response)
            | BootstrapAnswer::RetryLater(response),
        ) => response,
        Err(e) => {
            tracing::warn!(community = anchor, error = %e, "bootstrap request not answered — NAK");
            rekindle_transport::operations::direct::NAK.to_vec()
        }
    }
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
