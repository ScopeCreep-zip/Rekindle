//! The quarantine event (plan E3.2 S7c.5): the merge's quarantined ops
//! whose fate moved, announced once each.

use std::collections::HashMap;

use rekindle_governance::merge::MergeOutcome;
use rekindle_types::governance::OpHash;

use crate::event::GovernanceRuntimeEvent;
use crate::op_log::{HeldOp, OpFate};

/// One `GovernanceOpQuarantined` per op whose fate this merge moves to
/// quarantined with a reason (plan E3.2 S7c.5: after its author's removal,
/// two checkpoint endorsements in one round, checkpoint equivocation): an
/// op `held` under any other fate (or newly fetched) before the merge. An
/// op already held quarantined stays quiet, so each is shown once.
#[must_use]
pub fn newly_quarantined(
    community_id: &str,
    held: &[HeldOp],
    outcome: &MergeOutcome,
) -> Vec<GovernanceRuntimeEvent> {
    let by_hash: HashMap<OpHash, &HeldOp> = held.iter().map(|h| (h.hash, h)).collect();
    (outcome.quarantined.iter())
        .filter_map(|(hash, reason)| {
            let op = by_hash.get(hash)?;
            (op.fate != OpFate::Quarantined).then(|| {
                GovernanceRuntimeEvent::GovernanceOpQuarantined {
                    community_id: community_id.to_owned(),
                    op: *hash,
                    author: op.author.clone(),
                    reason: *reason,
                }
            })
        })
        .collect()
}
