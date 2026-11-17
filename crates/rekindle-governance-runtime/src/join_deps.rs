//! Shallow joins (plan E3.2 S7c.0, Step 0): the deps of a `MemberJoined`.
//!
//! Every other op cites the frontier ([`Deps::Frontier`](crate::op_log::Deps)).
//! A join citing the frontier depends on the previous joiner's
//! `MemberJoined` after a quiescent join, so the DAG's depth grows by one
//! per join. A join instead cites:
//!
//! - **the base**: the creator's last genesis op (seq
//!   [`GENESIS_LAST_SEQ`](crate::origin::GENESIS_LAST_SEQ), `ChannelCreated`;
//!   the `GenesisCert` is pinned outside the DAG, so no op stands for it).
//!   S7c.3/S7c.5 put a `Checkpoint` op of the joiner's hint here once
//!   checkpoint ops exist; none exist before them;
//! - **the inviter's latest op**, whose `prev` chain reaches the
//!   `InviteCreated` the invite gate needs (`gates::invite_gate`);
//! - on a re-join, **our previous op** (the chain rule: `prev` is among the
//!   deps) and **every op that strongly removes us** the walk found (a
//!   kick, a ban, a role taken away: `power::removal_target`), so the
//!   re-join follows them (`gates::rejoined` needs the removal in the
//!   re-join's past; concurrent, the re-join is invalidated with us);
//! - **the fold**: when the frontier the joiner walked holds more than
//!   [`FOLD_THRESHOLD`] heads (`MAX_FRONTIER_REFS / 2`), the newest heads by
//!   `(lamport, op_hash)` too, up to [`GOVERNANCE_OP_MAX_DEPS`] deps in all
//!   (62 with the base and the inviter's op). Heads past that stay heads
//!   and the next fold cites them. A fold at half the cap keeps every leaf
//!   join inside every published 64-ref frontier and grows depth by one
//!   per 33 joins, with no rule that someone must author a non-join op.
//!
//! Rules judged at an op's deps (threat model E7) see only this past for
//! a join: another admin's revocation outside the inviter's past leaves a
//! join concurrent with it and admitted, as a joiner choosing its deps
//! could already make it; a join concurrent with the inviter's removal is
//! still filtered through the transitive `InviteCreated` invalidation.

use std::collections::BTreeSet;

use rekindle_codec::governance_op::GOVERNANCE_OP_MAX_DEPS;
use rekindle_governance::op::VerifiedOp;
use rekindle_governance::power::removal_target;
use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;
use rekindle_types::governance::{GenesisCert, OpHash};
use rekindle_types::id::PseudonymKey;

use crate::error::GovernanceRuntimeError;
use crate::op_log::VerifiedSet;
use crate::origin::GENESIS_LAST_SEQ;

/// More heads than this in the walked frontier and a join folds them in.
pub const FOLD_THRESHOLD: usize = MAX_FRONTIER_REFS / 2;

/// The latest op of `author` in `ops` by `(seq, lamport, hash)`.
fn latest_of<'a>(ops: &'a [VerifiedOp], author: &PseudonymKey) -> Option<&'a VerifiedOp> {
    ops.iter()
        .filter(|op| op.author() == author)
        .max_by_key(|op| (op.op().seq, op.stamp()))
}

/// The deps of `joiner`'s `MemberJoined` on the invite of `inviter`, from
/// the walked set `set`, in canonical (hash) order.
///
/// # Errors
/// The walk holds no creator op at the last genesis seq, or no op of the
/// inviter.
pub fn join_deps(
    cert: &GenesisCert,
    set: &VerifiedSet,
    joiner: &PseudonymKey,
    inviter: &PseudonymKey,
) -> Result<Vec<OpHash>, GovernanceRuntimeError> {
    let base = set
        .ops
        .iter()
        .filter(|op| *op.author() == cert.creator_pseudonym && op.op().seq == GENESIS_LAST_SEQ)
        .min_by_key(|op| op.stamp())
        .ok_or_else(|| {
            GovernanceRuntimeError::Adapter(
                "join: the walk holds no creator op at the last genesis seq".into(),
            )
        })?;
    let invited = latest_of(&set.ops, inviter).ok_or_else(|| {
        GovernanceRuntimeError::Adapter("join: the walk holds no op of the inviter".into())
    })?;
    let mut cited: Vec<OpHash> = vec![base.hash(), invited.hash()];
    cited.extend(latest_of(&set.ops, joiner).map(VerifiedOp::hash));
    let mut removals: Vec<&VerifiedOp> = set
        .ops
        .iter()
        .filter(|op| op.author() != joiner && removal_target(op.body()) == Some(joiner))
        .collect();
    removals.sort_by_key(|op| std::cmp::Reverse(op.stamp()));
    cited.extend(removals.iter().map(|op| op.hash()));
    let mut seen: BTreeSet<OpHash> = BTreeSet::new();
    cited.retain(|h| seen.insert(*h));
    cited.extend(fold(
        set,
        &seen,
        GOVERNANCE_OP_MAX_DEPS.saturating_sub(cited.len()),
    ));
    cited.truncate(GOVERNANCE_OP_MAX_DEPS);
    cited.sort_unstable();
    Ok(cited)
}

/// The heads a fold join cites: none while the walked frontier holds at
/// most [`FOLD_THRESHOLD`] heads; else the newest `room` of them by
/// `(lamport, op_hash)`, other than those already `cited`.
fn fold(set: &VerifiedSet, cited: &BTreeSet<OpHash>, room: usize) -> Vec<OpHash> {
    let referenced: BTreeSet<OpHash> = set
        .ops
        .iter()
        .flat_map(|op| op.op().deps.iter().map(|d| d.op_hash))
        .collect();
    let mut heads: Vec<&VerifiedOp> = set
        .ops
        .iter()
        .filter(|op| !referenced.contains(&op.hash()))
        .collect();
    if heads.len() <= FOLD_THRESHOLD {
        return Vec::new();
    }
    heads.retain(|op| !cited.contains(&op.hash()));
    heads.sort_by_key(|op| std::cmp::Reverse(op.stamp()));
    heads.into_iter().take(room).map(VerifiedOp::hash).collect()
}

#[cfg(test)]
#[path = "join_deps_tests.rs"]
mod tests;
