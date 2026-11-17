//! The cut index and `stateRoot` (plan E3.2 S7c.3 part (2)): what a reader
//! needs, beside the merged state, to skip the cut's ops and reproduce every
//! post-cut fate. O(N + power ops), never O(history).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rekindle_types::domains;
use rekindle_types::governance::{OpHash, OpRef};

use crate::checksum::canon::{put_map, put_set, Canon};
use crate::checksum::canonical_state;
use crate::op::VerifiedOp;
use crate::state::GovernanceState;

/// The cut index of one cut.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CutIndex {
    /// Each author's highest seq in the cut, with that op's hash, keyed by
    /// pseudonym bytes: an op is in the cut iff its seq is at or below its
    /// author's entry.
    pub authors: BTreeMap<[u8; 32], (u64, OpHash)>,
    /// The cut's power ops in the order the power phase visits them, each
    /// with its fate (`true`: applied, `false`: refused).
    pub power: Vec<(OpHash, bool)>,
    /// The cut's power ops no other cut power op has in its causal past.
    pub power_heads: Vec<OpHash>,
    /// The greatest mainline key `(issued_at, op_hash)` among the cut's
    /// non-power ops, if it has any.
    pub mainline_max: Option<(u64, OpHash)>,
}

/// The canonical encoding of `cut`, under `canonical_state`'s rules.
#[must_use]
pub fn canonical_cut(cut: &CutIndex) -> Vec<u8> {
    let CutIndex {
        authors,
        power,
        power_heads,
        mainline_max,
    } = cut;
    let mut out = Vec::new();
    put_map(&mut out, authors);
    power.put(&mut out);
    put_set(&mut out, power_heads);
    mainline_max.put(&mut out);
    out
}

/// The preimage of a checkpoint's `stateRoot`, `canonical_state(state) ‖
/// canonical_cut(cut)`: the bytes a state snapshot carries (plan E3.2
/// S7c.3, "State snapshot"). The `Frontier`'s roster checksum hashes the
/// first part alone, so the two share one encoder while the cut index,
/// which differs between readers holding different op sets, stays out of
/// D15.
#[must_use]
pub fn state_preimage(state: &GovernanceState, cut: &CutIndex) -> Vec<u8> {
    let mut out = canonical_state(state);
    out.extend_from_slice(&canonical_cut(cut));
    out
}

/// BLAKE3 `derive_key(GOV_CHECKPOINT_STATE)` over a `stateRoot` preimage,
/// which a reader checks a reassembled snapshot with before decoding it.
#[must_use]
pub fn state_root_of(preimage: &[u8]) -> [u8; 32] {
    blake3::derive_key(domains::GOV_CHECKPOINT_STATE, preimage)
}

/// A checkpoint's `stateRoot`: [`state_root_of`] its [`state_preimage`].
#[must_use]
pub fn state_root(state: &GovernanceState, cut: &CutIndex) -> [u8; 32] {
    state_root_of(&state_preimage(state, cut))
}

/// The ops `frontier` reaches through deps (and `prev`), the frontier
/// included, in the set's order: a cut's ops when `frontier` is its heads.
/// A frontier op not held is left out, as is everything only it reaches.
#[must_use]
pub fn causal_past(ops: &[VerifiedOp], frontier: &[OpRef]) -> Vec<VerifiedOp> {
    let by_hash: HashMap<OpHash, &VerifiedOp> = ops.iter().map(|o| (o.hash(), o)).collect();
    let mut reached: BTreeSet<OpHash> = BTreeSet::new();
    let mut stack: Vec<OpHash> = frontier.iter().map(|r| r.op_hash).collect();
    while let Some(hash) = stack.pop() {
        let Some(op) = by_hash.get(&hash) else {
            continue;
        };
        if reached.insert(hash) {
            stack.extend(op.op().deps.iter().map(|d| d.op_hash));
            stack.extend(op.op().prev);
        }
    }
    ops.iter()
        .filter(|o| reached.contains(&o.hash()))
        .cloned()
        .collect()
}
