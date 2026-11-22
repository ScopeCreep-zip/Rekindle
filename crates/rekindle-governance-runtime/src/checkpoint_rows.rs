//! The checkpoint rows a rebuild writes (plan E3.2 S7c.6): every
//! (height, round, digest) an applied `Checkpoint` op carries, with its
//! state in the merge: `Usable` once a strict majority of its electorate
//! endorses it, `Equivocated` when it is one of two majority digests for
//! one cut and one `prevCheckpoint`, else `Proposed`. Which hint is selected
//! is computed from the op set (`Hints::selected`), never stored.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_governance::checkpoint::checkpoint_digest;
use rekindle_governance::merge::MergeOutcome;
use rekindle_governance::op::VerifiedOp;
use rekindle_types::governance::{GovernanceEntry, OpHash};

/// A checkpoint digest's state after a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintState {
    Proposed,
    Usable,
    Equivocated,
}

/// One (height, round, digest) and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointFact {
    pub height: u64,
    pub round: u32,
    pub digest: [u8; 32],
    /// The lowest `(lamport, op_hash)` applied op carrying it.
    pub op: OpHash,
    pub dir_size: u64,
    pub state: HintState,
}

/// The facts of every digest the `Checkpoint` ops among `ops` carry that
/// the merge applied or holds as an endorsement fault, in `(height, round, digest)` order, with their state in
/// `outcome` (the merge of `ops`).
#[must_use]
pub fn checkpoint_facts(ops: &[VerifiedOp], outcome: &MergeOutcome) -> Vec<CheckpointFact> {
    // The merge quarantines an endorsement that counts for nothing (two in
    // one round, an equivocation): its digest keeps its row, so a digest
    // usable before is marked equivocated, never left usable.
    let applied: BTreeSet<OpHash> = (outcome.applied.iter().copied())
        .chain(outcome.hints.faults.iter().map(|(hash, _)| *hash))
        .collect();
    let equivocated: BTreeSet<[u8; 32]> = (outcome.hints.equivocations.iter())
        .flat_map(|q| q.digests.iter().copied())
        .collect();
    let mut lowest: BTreeMap<(u64, u32, [u8; 32]), (&VerifiedOp, u64)> = BTreeMap::new();
    for op in ops.iter().filter(|op| applied.contains(&op.hash())) {
        let GovernanceEntry::Checkpoint(body) = op.body() else {
            continue;
        };
        let key = (body.height, body.round, checkpoint_digest(body));
        let entry = lowest.entry(key).or_insert((op, body.dir_size));
        if op.stamp() < entry.0.stamp() {
            *entry = (op, body.dir_size);
        }
    }
    lowest
        .into_iter()
        .map(|((height, round, digest), (op, dir_size))| CheckpointFact {
            height,
            round,
            digest,
            op: op.hash(),
            dir_size,
            state: if equivocated.contains(&digest) {
                HintState::Equivocated
            } else if outcome.hints.is_usable(height, round, &digest) {
                HintState::Usable
            } else {
                HintState::Proposed
            },
        })
        .collect()
}

#[cfg(test)]
#[path = "checkpoint_rows_tests.rs"]
mod tests;

/// Store a merge's checkpoint facts and keep the latest endorsed checkpoint's
/// directory pages warm (plan E3.2 S7: record warming).
///
/// # Errors
/// The deps' failure to store the facts.
pub async fn record<D: crate::deps::GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    ops: &[VerifiedOp],
    outcome: &MergeOutcome,
) -> Result<(), crate::error::GovernanceRuntimeError> {
    deps.op_store()
        .record_governance_checkpoints(community_id, checkpoint_facts(ops, outcome))
        .await?;
    if let Some(latest) = outcome.hints.latest() {
        deps.warm_checkpoint_pages(community_id, &latest.checkpoint);
    }
    Ok(())
}
