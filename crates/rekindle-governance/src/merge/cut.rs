//! The merge of a checkpoint's cut (plan E3.2 S7c.3): the state at the cut
//! and its cut index, from one resolution.

use std::collections::BTreeMap;

use rekindle_types::governance::{GenesisCert, OpHash};

use super::{resolve, resolve_stable};
use crate::checkpoint::CutIndex;
use crate::op::VerifiedOp;
use crate::power::{class_of, OpClass};
use crate::state::GovernanceState;

/// Merge `cut_ops`, the causal closure of a checkpoint's cut, under `cert`:
/// the state at the cut and its cut index, which `stateRoot` digests
/// (`checkpoint::state_root`).
#[must_use]
pub fn merge_cut(cert: &GenesisCert, cut_ops: &[VerifiedOp]) -> (GovernanceState, CutIndex) {
    let set = resolve::prepare(cut_ops);
    let round = resolve_stable(cert, &set);
    let mut authors: BTreeMap<[u8; 32], (u64, OpHash)> = BTreeMap::new();
    for op in &set.ops {
        let entry = (op.op().seq, op.hash());
        (authors.entry(op.author().0))
            .and_modify(|held| *held = (*held).max(entry))
            .or_insert(entry);
    }
    let n = set.ops.len();
    let is_power = |i: usize| class_of(set.ops[i].body()) == OpClass::Power;
    let power_idx: Vec<usize> = (0..n).filter(|&i| is_power(i)).collect();
    let power = power_idx
        .iter()
        .map(|&i| (set.ops[i].hash(), round.applied[i]))
        .collect();
    let mut power_heads: Vec<OpHash> = power_idx
        .iter()
        .filter(|&&i| !power_idx.iter().any(|&j| set.past.contains(j, i)))
        .map(|&i| set.ops[i].hash())
        .collect();
    power_heads.sort_unstable();
    let mainline_max = (0..n)
        .filter(|&i| !is_power(i))
        .map(|i| (set.ops[i].op().issued_at, set.ops[i].hash()))
        .max();
    let cut = CutIndex {
        authors,
        power,
        power_heads,
        mainline_max,
    };
    (round.state, cut)
}
