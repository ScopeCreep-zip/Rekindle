//! Quarantined ops as ancestry (ADR 0022 rule 5, plan E3.2 S7d-keys.2a):
//! the authority cascade and the one-way rule for refusals.
//!
//! An equivocating author's ops from its fork on (`OpSet::forked`) stay in
//! the DAG and are never applied. Two resolutions decide what that does to
//! the ops built on them, both pure functions of the op set (D15):
//!
//! - **counted**: every forked op that passes `permitted` at its deps
//!   counted as applied, as if the fork were not known;
//! - **final**: the forked ops left out, with the refusals below kept.
//!
//! **One-way for refusals:** an op the counted round does not apply while
//! a forked op `q` applied there stood (`q` in its causal past, or `q` a
//! removal whose target seq frontier masks it) stays refused in the final
//! round unless its own causal past holds `q`'s quarantine cause, so
//! nothing members saw refused is revived without its author acting.
//!
//! **The authority cascade:** an op the counted round applies and the final
//! round does not, with a forked op applied in the counted round in its
//! causal past, lost its authority with that op: it is quarantined, its
//! cause the lowest `(lamport, op_hash)` of those ops (its authority path).
//! One whose authority holds through another path stays applied.

use std::collections::BTreeMap;

use rekindle_types::governance::GenesisCert;

use super::resolve::OpSet;
use super::{resolve_stable, Settled};

/// The final resolution and, per op the cascade quarantined, its cause.
pub(super) struct Settlement {
    pub settled: Settled,
    /// Op index to the index of the quarantined op its authority came from.
    pub cascade: BTreeMap<usize, usize>,
}

/// Resolve `set` with its forked ops kept as ancestry.
pub(super) fn settle(cert: &GenesisCert, set: &OpSet<'_>) -> Settlement {
    let n = set.ops.len();
    // An op whose device key is refused for its author is ancestry only in
    // both rounds (ADR 0022 rule 6, `merge::judge`).
    let judged_out = set.judged_out();
    if !set.forked.contains(&true) {
        return Settlement {
            settled: resolve_stable(cert, set, &judged_out),
            cascade: BTreeMap::new(),
        };
    }
    let (counted, counted_mask, _, _) = resolve_stable(cert, set, &judged_out);
    let stood = |q: usize| set.forked[q] && counted.applied[q];
    let in_past = |i: usize| (0..i).filter(move |&q| stood(q) && set.past.contains(i, q));
    let held: Vec<bool> = (0..n)
        .map(|i| {
            if set.forked[i] || counted.applied[i] {
                return false;
            }
            let mask = counted_mask[i].filter(|&q| stood(q));
            (in_past(i).chain(mask))
                .any(|q| set.fork_cause[q].is_none_or(|c| !set.past.contains(i, c)))
        })
        .collect();
    let exclude: Vec<bool> = (0..n)
        .map(|i| set.forked[i] || held[i] || judged_out[i])
        .collect();
    let settled = resolve_stable(cert, set, &exclude);
    let (round, masked_by, _, record_refusals) = &settled;
    let cascade = (0..n)
        .filter(|&i| !exclude[i] && counted.applied[i] && !round.applied[i])
        .filter(|&i| masked_by[i].is_none() && !record_refusals.contains_key(&i))
        .filter_map(|i| in_past(i).next().map(|q| (i, q)))
        .collect();
    Settlement { settled, cascade }
}
