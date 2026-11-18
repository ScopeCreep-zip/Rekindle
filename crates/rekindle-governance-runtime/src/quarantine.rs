//! The quarantine events: the merge's quarantined ops whose fate moved
//! (plan E3.2 S7c.5), and the held ops a member signed that this reader
//! cannot interpret, which quarantine the community (D15); each announced
//! once.

use std::collections::HashMap;

use rekindle_codec::governance_op::{read_op, ReadOp};
use rekindle_governance::merge::MergeOutcome;
use rekindle_governance::op::{op_hash, op_signature_holds};
use rekindle_governance::state::GovernanceState;
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

/// One `CommunityQuarantined` per held op not yet held quarantined that
/// this reader cannot interpret (D15: an op kind or version it does not
/// know quarantines the community with a visible reason, never applied in
/// part, never dropped silently) and that a current member of `state`
/// signed for community `community_id`. The merge never sees such an op
/// (it does not decode, so it is held quarantined, `fates`), the state is
/// the merge without it, and an op citing it waits on it. One whose author
/// is not a member, or whose signature fails, is held quarantined without
/// the community notice: nobody outside the community can quarantine it.
#[must_use]
pub fn newly_uninterpretable(
    community_id: &str,
    held: &[HeldOp],
    state: &GovernanceState,
) -> Vec<GovernanceRuntimeEvent> {
    (held.iter().filter(|h| h.fate != OpFate::Quarantined))
        .filter_map(|h| {
            let Ok(ReadOp::Unknown(op)) = read_op(&h.encoded) else {
                return None;
            };
            let authentic = op_hash(&op.unsigned, &op.sig) == h.hash
                && op.author == h.author
                && op_signature_holds(community_id, &op.author, &op.unsigned, &op.sig);
            if !authentic || !state.members.contains_key(&op.author) {
                tracing::warn!(
                    community = community_id,
                    op = %hex::encode(h.hash.0),
                    "an uninterpretable governance op not signed by a member is held quarantined"
                );
                return None;
            }
            tracing::warn!(
                community = community_id,
                op = %hex::encode(h.hash.0),
                author = %hex::encode(op.author.0),
                what = ?op.what,
                "a member's governance op this version cannot interpret quarantines the community"
            );
            Some(GovernanceRuntimeEvent::CommunityQuarantined {
                community_id: community_id.to_owned(),
                op: h.hash,
                author: op.author,
                what: op.what,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "quarantine_tests.rs"]
mod tests;
