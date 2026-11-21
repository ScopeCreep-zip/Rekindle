//! The invalidated set's fixpoint, shared by the full merge
//! (`resolve_stable`) and the resumed one (`resume::merge_from`), so both
//! settle the same way over the same ops.
//!
//! A round leaves out the invalid ops and the gates judge the round
//! (strong removal, approval, invites, records). The result is the next
//! invalid set, until that set is stable. Masking is transitive (a masked
//! removal reinstates its target's ops), so the sequence need not grow
//! monotonically. It can also cycle. Example: a removal whose authority
//! rests on its target's concurrent self-demotion, which the removal
//! itself masks: with the demotion the removal applies and masks the
//! demotion; without it the removal lacks authority and masks nothing.
//! A cycle never settles. Stopping it at a pass cap would pick a round by
//! the parity of the op count, which differs between a full replay and a
//! resume from a checkpoint. It would also leave an op both applied and
//! quarantined. So a cycle settles on the union of its invalid sets: an
//! op that any round of the cycle invalidated stays out (no op stands on
//! an authority that is circular, the threat model's no-creative-paths
//! rule). The set then widens monotonically, adding whatever the gates
//! invalidate, until it is stable. That ends within `n` more passes.
//! The result depends on the ops alone, never on how many there are.

/// Runs `round` and `gate` until the invalid set is stable, as above;
/// returns the settled round and its gates' verdict. `invalid_of` reads
/// the invalid set a verdict carries.
pub(super) fn settle<R, G, E>(
    n: usize,
    mut round: impl FnMut(&[bool]) -> R,
    mut gate: impl FnMut(&R) -> Result<G, E>,
    invalid_of: impl Fn(&G) -> &[bool],
) -> Result<(R, G), E> {
    let mut invalid = vec![false; n];
    let mut seen: Vec<Vec<bool>> = Vec::new();
    let mut widening = false;
    loop {
        let settled = round(&invalid);
        let verdict = gate(&settled)?;
        let next = invalid_of(&verdict);
        if widening {
            let wider = union(&invalid, next);
            if wider == invalid {
                return Ok((settled, verdict));
            }
            invalid = wider;
            continue;
        }
        if next == invalid.as_slice() {
            return Ok((settled, verdict));
        }
        seen.push(std::mem::replace(&mut invalid, next.to_vec()));
        if let Some(start) = seen.iter().position(|s| *s == invalid) {
            invalid = seen[start..]
                .iter()
                .fold(vec![false; n], |acc, s| union(&acc, s));
            widening = true;
        }
    }
}

fn union(a: &[bool], b: &[bool]) -> Vec<bool> {
    a.iter().zip(b).map(|(x, y)| *x || *y).collect()
}
