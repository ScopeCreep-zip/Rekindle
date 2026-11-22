//! A community joined from a checkpoint hint is partial until its backfill
//! ends (plan E3.2 S7c.8 (1); Synapse `is_partial_state_room`,
//! `federation_server.py`, and `event_auth.py` `assert_host_in_room`'s
//! `UNABLE_DUE_TO_PARTIAL_STATE`; MSC3902 §Resynchronisation).
//!
//! The joiner's hinted walk (`join_walk`) holds only the ops above the
//! hint's cut; its normal rebuild (`op_log::rebuild_ops`) then fetches every
//! cut op they cite until no dep is missing. Until then its op set is not
//! the community's, so it proposes and endorses no checkpoint
//! (`checkpointing::tick`), answers no hint, snapshot or bootstrap request
//! ([`refuse_while_partial`]) and compares no D15 roster checksum
//! (`anti_entropy`). The flag is durable (`governance_partial`), so a
//! restart mid-backfill stays partial. When the backfill ends
//! ([`finish_backfill`]) the full op set's state at the cut is compared
//! with the hinted snapshot's digest; a mismatch is a false snapshot
//! (threat T8), published as a typed event, and the community stays on the
//! backfilled state, the merge of every op it holds.

use rekindle_governance::checkpoint::{checkpoint_digest, state_preimage, state_root_of};
use rekindle_governance::merge::merge_cut;
use rekindle_types::governance::{GenesisCert, GovernanceEntry};

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::op_log::{causal_past, VerifiedSet};

/// The hint a partial community was joined from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialJoin {
    /// The hint's checkpoint height.
    pub height: u64,
    /// The hint's checkpoint digest.
    pub digest: [u8; 32],
    /// The hinted snapshot's digest: the checkpoint's `stateRoot`, which
    /// the snapshot reproduced before the walk resumed from it.
    pub state_root: [u8; 32],
}

/// Whether `community_id` is partial now. A failed read counts as partial:
/// a node that cannot tell refuses rather than serve a state it cannot
/// vouch for.
pub async fn is_partial<D: GovernanceRuntimeDeps>(deps: &D, community_id: &str) -> bool {
    match deps.op_store().governance_partial(community_id).await {
        Ok(hint) => hint.is_some(),
        Err(error) => {
            tracing::warn!(community = community_id, %error, "partial flag unreadable: held partial");
            true
        }
    }
}

/// `Err(PartialState)` while `community_id` is partial: the responders'
/// gate (hint, snapshot and bootstrap requests).
///
/// # Errors
/// The community is partial.
pub async fn refuse_while_partial<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<(), GovernanceRuntimeError> {
    if is_partial(deps, community_id).await {
        return Err(GovernanceRuntimeError::PartialState(
            community_id.to_owned(),
        ));
    }
    Ok(())
}

/// After a rebuild's merge of `set`: when `community_id` is partial and
/// `set` misses no dep, the backfill has ended. The full op set's state at
/// the hint's cut is compared with the hinted snapshot's digest, a
/// mismatch publishes [`GovernanceRuntimeEvent::CheckpointSnapshotMismatch`],
/// and the community is marked whole either way. A set still missing a dep
/// (an author's log unreadable now) stays partial for the next rebuild.
///
/// # Errors
/// The store failed.
pub async fn finish_backfill<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    cert: &GenesisCert,
    set: &VerifiedSet,
) -> Result<(), GovernanceRuntimeError> {
    let Some(hint) = deps.op_store().governance_partial(community_id).await? else {
        return Ok(());
    };
    if !set.missing.is_empty() || !set.pending.is_empty() {
        return Ok(());
    }
    let checkpoint = set.ops.iter().find_map(|op| match op.body() {
        GovernanceEntry::Checkpoint(c)
            if c.height == hint.height && checkpoint_digest(c) == hint.digest =>
        {
            Some(c)
        }
        _ => None,
    });
    let Some(checkpoint) = checkpoint else {
        // The join cites the hint's lowest op, so it is held once the join
        // is: without it the set is not the one the join walked.
        return Ok(());
    };
    let (state, cut) = merge_cut(cert, &causal_past(&set.ops, &checkpoint.cut));
    if state_root_of(&state_preimage(&state, &cut)) != hint.state_root {
        tracing::warn!(
            community = community_id,
            height = hint.height,
            "backfill ended: the hinted snapshot is not the op set's state at its cut"
        );
        deps.emit_event(GovernanceRuntimeEvent::CheckpointSnapshotMismatch {
            community_id: community_id.to_owned(),
            height: hint.height,
            digest: hint.digest,
        });
    }
    deps.op_store()
        .set_governance_partial(community_id, None)
        .await
}
