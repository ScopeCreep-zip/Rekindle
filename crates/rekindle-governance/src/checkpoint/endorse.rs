//! When to propose and what to endorse (plan E3.2 S7c.4). Both triggers
//! are local and no reader checks them: a wrong, stale or late hint costs
//! only a longer walk, so no liveness or finality rule is needed.

use std::collections::HashSet;

use rekindle_types::governance::{Checkpoint, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::cut::causal_past;
use super::shape::checkpoint_digest;
use crate::merge::MergeOutcome;
use crate::op::VerifiedOp;
use crate::power::{class_of, OpClass};

/// Applied ops beyond the previous hint's cut that make a proposal due.
pub const PROPOSAL_OPS: usize = 128;

/// What lies beyond the previous hint's cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Beyond {
    /// Applied ops outside the cut, `Checkpoint` ops aside.
    pub applied: usize,
    /// Whether a power op is among them.
    pub power: bool,
}

/// What `outcome` applied beyond `prev`'s cut (all of it with no `prev`).
#[must_use]
pub fn beyond(ops: &[VerifiedOp], outcome: &MergeOutcome, prev: Option<&Checkpoint>) -> Beyond {
    let inside: HashSet<OpHash> = prev
        .map(|p| {
            causal_past(ops, &p.cut)
                .iter()
                .map(VerifiedOp::hash)
                .collect()
        })
        .unwrap_or_default();
    let applied: HashSet<OpHash> = outcome.applied.iter().copied().collect();
    let mut out = Beyond::default();
    for op in ops {
        let hash = op.hash();
        if inside.contains(&hash)
            || !applied.contains(&hash)
            || matches!(op.body(), GovernanceEntry::Checkpoint(_))
        {
            continue;
        }
        out.applied += 1;
        out.power |= class_of(op.body()) == OpClass::Power;
    }
    out
}

/// Whether a proposal is due: [`PROPOSAL_OPS`] applied ops beyond the
/// previous cut, or a power op beyond it once one `SWEEP_PERIOD` passed
/// with no newer op (`quiet_for_a_sweep`, the caller's clock), so that a
/// hint soon covers the last power op.
#[must_use]
pub fn proposal_due(beyond: Beyond, quiet_for_a_sweep: bool) -> bool {
    beyond.applied >= PROPOSAL_OPS || (beyond.power && quiet_for_a_sweep)
}

/// The applied `Checkpoint` ops at `height`, lowest `(lamport, op_hash)`
/// first: the proposals an endorser chooses among.
#[must_use]
pub fn proposals<'a>(
    ops: &'a [VerifiedOp],
    outcome: &MergeOutcome,
    height: u64,
) -> Vec<(&'a VerifiedOp, &'a Checkpoint)> {
    let applied: HashSet<OpHash> = outcome.applied.iter().copied().collect();
    let mut out: Vec<(&VerifiedOp, &Checkpoint)> = (ops.iter())
        .filter(|o| applied.contains(&o.hash()))
        .filter_map(|o| match o.body() {
            GovernanceEntry::Checkpoint(c) if c.height == height => Some((o, c)),
            _ => None,
        })
        .collect();
    out.sort_by_key(|(o, _)| (o.lamport(), o.hash()));
    out
}

/// The `Checkpoint` body `me` signs now at `height`, if any. An
/// endorsement is write-once per (height, round): with none yet, round 0
/// endorses the lowest proposal `recomputes` confirms (our replay at its
/// cut gives the identical body); once our round-r endorsement has not
/// reached a strict majority within one `SWEEP_PERIOD`
/// (`round_timed_out`, the caller's clock), round r + 1 endorses the
/// lowest proposal we hold and recomputed. The electorate and the
/// majority stay those of the digest's `prevCheckpoint`: a timeout never
/// lowers them.
#[must_use]
pub fn endorsement(
    me: &PseudonymKey,
    ops: &[VerifiedOp],
    outcome: &MergeOutcome,
    height: u64,
    round_timed_out: bool,
    recomputes: impl Fn(&Checkpoint) -> bool,
) -> Option<Checkpoint> {
    let mine: Vec<&Checkpoint> = (ops.iter())
        .filter(|o| o.author() == me)
        .filter_map(|o| match o.body() {
            GovernanceEntry::Checkpoint(c) if c.height == height => Some(c),
            _ => None,
        })
        .collect();
    let round = match mine.iter().map(|c| c.round).max() {
        None => 0,
        Some(r) => {
            let reached = (mine.iter().filter(|c| c.round == r))
                .any(|c| (outcome.hints).is_usable(height, r, &checkpoint_digest(c)));
            if reached || !round_timed_out {
                return None;
            }
            r.checked_add(1)?
        }
    };
    let (_, chosen) = proposals(ops, outcome, height)
        .into_iter()
        .find(|(_, body)| recomputes(body))?;
    Some(Checkpoint {
        round,
        ..chosen.clone()
    })
}
