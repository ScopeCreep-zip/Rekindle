//! The governance frontier captured by a channel writer and replayed by its readers.

use rekindle_governance::{merge, state::GovernanceState};
use rekindle_types::governance::OpRef;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::op_log::{causal_past, frontier_refs, verify_held_with};

/// Capture the verified governance frontier for a post, or fail if its heads
/// cannot represent all of the verified governance we hold.
///
/// # Errors
/// The genesis, projection, store or complete frontier is unavailable.
pub async fn capture_post_cut<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<Vec<OpRef>, GovernanceRuntimeError> {
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_owned()))?;
    let held = deps.op_store().governance_ops(community_id).await?;
    let set = verify_held_with(deps, community_id, &held);
    let cut = frontier_refs(&cert, &state, &set);
    if cut.is_empty() || causal_past(&set.ops, &cut).len() != set.ops.len() {
        return Err(GovernanceRuntimeError::OpRefused(
            "incomplete post governance frontier".into(),
        ));
    }
    Ok(cut)
}

/// Replay a post's cut once every cited head is verified along with its
/// dependencies. A partial causal past is never an authorization state.
///
/// # Errors
/// The genesis or op store is unavailable.
pub async fn resolve_post_cut<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    frontier: &[OpRef],
) -> Result<Option<GovernanceState>, GovernanceRuntimeError> {
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let held = deps.op_store().governance_ops(community_id).await?;
    let set = verify_held_with(deps, community_id, &held);
    let past = causal_past(&set.ops, frontier);
    let complete = frontier.iter().all(|head| {
        past.iter().any(|op| {
            op.hash() == head.op_hash
                && op.author() == &head.author
                && op.op().device_key == head.device_key
                && op.op().seq == head.author_seq
        })
    });
    Ok(complete.then(|| merge::merge(&cert, &past)))
}
