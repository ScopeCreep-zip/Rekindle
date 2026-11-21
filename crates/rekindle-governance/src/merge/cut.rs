//! The merge of a checkpoint's cut (plan E3.2 S7c.3): the state at the cut
//! and its cut index, from one resolution.

use std::collections::BTreeMap;

use rekindle_types::governance::{GenesisCert, GovernanceEntry, OpHash};

use super::{ancestry, resolve, Round};
use crate::checkpoint::{CutIndex, CutOp};
use crate::op::VerifiedOp;
use crate::power::{class_of, removal_target, target_seq_frontier, OpClass};
use crate::state::GovernanceState;

/// Merge `cut_ops`, the causal closure of a checkpoint's cut, under `cert`:
/// the state at the cut and its cut index, which `stateRoot` digests
/// (`checkpoint::state_root`).
#[must_use]
pub fn merge_cut(cert: &GenesisCert, cut_ops: &[VerifiedOp]) -> (GovernanceState, CutIndex) {
    let set = resolve::prepare(cut_ops);
    let (round, _, _, _) = ancestry::settle(cert, &set).settled;
    // A forked op is ancestry only (ADR 0022 rule 5): the cut index counts
    // the merged ops, and `forks` names the fork.
    let merged = |i: &usize| !set.forked[*i];
    let mut authors: BTreeMap<[u8; 32], (u64, OpHash)> = BTreeMap::new();
    let mut lamports: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    for op in (0..set.ops.len()).filter(merged).map(|i| set.ops[i]) {
        let entry = (op.op().seq, op.hash());
        if authors.get(&op.author().0).is_none_or(|held| *held < entry) {
            authors.insert(op.author().0, entry);
            lamports.insert(op.author().0, op.lamport());
        }
    }
    let n = set.ops.len();
    let is_power = |i: usize| class_of(set.ops[i].body()) == OpClass::Power;
    let power_idx: Vec<usize> = (0..n).filter(merged).filter(|&i| is_power(i)).collect();
    let power = power_idx
        .iter()
        .map(|&i| (CutOp::of(set.ops[i]), round.applied[i]))
        .collect();
    let mut power_heads: Vec<OpHash> = power_idx
        .iter()
        .filter(|&&i| !power_idx.iter().any(|&j| set.past.contains(j, i)))
        .map(|&i| set.ops[i].hash())
        .collect();
    power_heads.sort_unstable();
    let mainline_max = (0..n)
        .filter(merged)
        .filter(|&i| !is_power(i))
        .map(|i| (set.ops[i].op().issued_at, set.ops[i].hash()))
        .max();
    let authors_live = authors.clone();
    let cut = CutIndex {
        authors,
        power,
        power_heads,
        mainline_max,
        holders: holders(cert, &set, &round),
        masked: masked(&set, &round),
        invite_codes: invite_codes(&set, &round),
        forks: forks(cut_ops),
        unmerged: unmerged(cut_ops, &authors_live),
        lamports,
        checkpoints: checkpoints(&set, &round),
    };
    (round.state, cut)
}

/// `record_gate`'s holder map after the settled round: the cert's creator,
/// then the first applied join or move naming each record, in stamp order.
fn holders(
    cert: &GenesisCert,
    set: &resolve::OpSet<'_>,
    round: &Round,
) -> BTreeMap<String, [u8; 32]> {
    let mut out = BTreeMap::new();
    out.insert(cert.creator_member_record.clone(), cert.creator_pseudonym.0);
    for (_, op) in set
        .ops
        .iter()
        .enumerate()
        .filter(|&(o, _)| round.applied[o])
    {
        if let GovernanceEntry::MemberJoined { member_record, .. }
        | GovernanceEntry::MemberRecordMoved { member_record, .. } = op.body()
        {
            out.entry(member_record.clone()).or_insert(op.author().0);
        }
    }
    out
}

/// The targets of applied removals (`MemberRemoved`, `BanEntry`, demotion;
/// never of the remover itself) that no applied re-join of the target with
/// the removal in its causal past follows: every later op of theirs that
/// has the whole cut in its causal past is masked (`gates::strong_removal`,
/// whose `rejoined` exemption is the only one such an op can have). Each
/// maps to the first of those removals in stamp order carrying a seq
/// frontier, the cause `merge_outcome` quarantines under.
fn masked(set: &resolve::OpSet<'_>, round: &Round) -> BTreeMap<[u8; 32], Option<OpHash>> {
    let mut out: BTreeMap<[u8; 32], Option<OpHash>> = BTreeMap::new();
    for (r, op) in set
        .ops
        .iter()
        .enumerate()
        .filter(|&(r, _)| round.applied[r])
    {
        let Some(target) = removal_target(op.body()).filter(|t| *t != op.author()) else {
            continue;
        };
        let lifted = (0..set.ops.len()).any(|j| {
            round.applied[j]
                && set.ops[j].author() == target
                && matches!(set.ops[j].body(), GovernanceEntry::MemberJoined { .. })
                && set.past.contains(j, r)
        });
        if lifted {
            continue;
        }
        let cause = out.entry(target.0).or_insert(None);
        if cause.is_none() && target_seq_frontier(op.body()).is_some() {
            *cause = Some(op.hash());
        }
    }
    out
}

/// Per code hash, the first applied `InviteCreated` carrying it (in stamp
/// order, `gates::invite_gate`'s search order), whether an applied
/// `InviteRevoked` names it, and its locator.
fn invite_codes(
    set: &resolve::OpSet<'_>,
    round: &Round,
) -> BTreeMap<String, ([u8; 16], bool, CutOp)> {
    let applied = || (0..set.ops.len()).filter(|&i| round.applied[i]);
    let mut out = BTreeMap::new();
    for c in applied() {
        if let GovernanceEntry::InviteCreated {
            invite_id,
            code_hash,
            ..
        } = set.ops[c].body()
        {
            let revoked = applied().any(|v| {
                matches!(set.ops[v].body(), GovernanceEntry::InviteRevoked { invite_id: id } if id == invite_id)
            });
            out.entry(code_hash.clone())
                .or_insert((*invite_id, revoked, CutOp::of(set.ops[c])));
        }
    }
    out
}

/// The applied `Checkpoint` ops at the cut's two greatest applied heights,
/// with their heights.
fn checkpoints(set: &resolve::OpSet<'_>, round: &Round) -> BTreeMap<CutOp, u64> {
    let applied: Vec<(CutOp, u64)> = (set.ops.iter().enumerate())
        .filter(|&(o, _)| round.applied[o])
        .filter_map(|(_, op)| match op.body() {
            GovernanceEntry::Checkpoint(c) => Some((CutOp::of(op), c.height)),
            _ => None,
        })
        .collect();
    let top = applied.iter().map(|(_, h)| *h).max().unwrap_or(0);
    (applied.into_iter())
        .filter(|(_, h)| h.saturating_add(1) >= top)
        .collect()
}

/// Each author that signed two ops at one seq, with the lowest such seq
/// (`resolve::prepare`'s `fork_at`).
fn forks(cut_ops: &[VerifiedOp]) -> BTreeMap<[u8; 32], u64> {
    let mut seen: BTreeMap<([u8; 32], u64), OpHash> = BTreeMap::new();
    let mut out: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    for op in cut_ops {
        let key = (op.author().0, op.op().seq);
        if seen.insert(key, op.hash()).is_some_and(|h| h != op.hash()) {
            let at = out.entry(key.0).or_insert(u64::MAX);
            *at = (*at).min(key.1);
        }
    }
    out
}

/// Each author with a cut op above its merged ops (`authors`), with the
/// highest seq of any of its cut ops.
fn unmerged(
    cut_ops: &[VerifiedOp],
    live: &BTreeMap<[u8; 32], (u64, OpHash)>,
) -> BTreeMap<[u8; 32], u64> {
    let mut out: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    for op in cut_ops {
        let author = op.author().0;
        if live.get(&author).is_none_or(|(s, _)| op.op().seq > *s) {
            let top = out.entry(author).or_insert(0);
            *top = (*top).max(op.op().seq);
        }
    }
    out
}
