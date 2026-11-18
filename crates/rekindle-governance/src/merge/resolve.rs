//! Op-set preparation for `merge`: dedup, equivocation, causal completeness
//! and the ancestry every resolution rule asks about (plan E3.2).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

use rekindle_types::governance::OpHash;
use rekindle_types::id::PseudonymKey;

use crate::op::VerifiedOp;

/// The op set `merge` resolves: complete (every dep present), free of
/// equivocating forks, in stamp order (which is causal: a dep's lamport is
/// below its dependant's), with each op's causal past.
pub(super) struct OpSet<'a> {
    pub ops: Vec<&'a VerifiedOp>,
    pub past: Ancestry,
    /// Ops whose deps are missing from the set (`pending_deps`).
    pub pending: Vec<OpHash>,
    /// Authors that signed two different ops at one seq, by key bytes.
    pub equivocators: Vec<PseudonymKey>,
}

/// `past.contains(i, j)`: op `j` is in op `i`'s causal past.
///
/// A dep is always earlier in stamp order than its dependant, so op `i`'s
/// past holds only ops below `i`: row `i` stores the bits for `0..i` alone
/// (a lower-triangular bit matrix). Rows are built in stamp order on the
/// first query that reaches them, so a merge pays only for the prefix of
/// the matrix its rules ask about (all of it once a late op is queried).
pub(super) struct Ancestry {
    /// Each op's deps, latest first, flattened: op `i`'s are
    /// `deps[dep_at[i]..dep_at[i + 1]]`.
    deps: Vec<usize>,
    dep_at: Vec<usize>,
    /// Where row `i`'s words start in `bits`; `row[i + 1]` is where it ends.
    row: Vec<usize>,
    /// Rows `0..built`, in order.
    bits: RefCell<Vec<u64>>,
    built: Cell<usize>,
}

impl Ancestry {
    pub(super) fn with_capacity(n: usize) -> Self {
        let mut row = Vec::with_capacity(n + 1);
        row.push(0);
        let mut dep_at = Vec::with_capacity(n + 1);
        dep_at.push(0);
        Self {
            deps: Vec::with_capacity(2 * n),
            dep_at,
            row,
            bits: RefCell::new(Vec::new()),
            built: Cell::new(0),
        }
    }

    /// Records the next op (in stamp order) and its deps.
    pub(super) fn push(&mut self, deps: &[usize]) {
        let i = self.dep_at.len() - 1;
        let from = self.deps.len();
        self.deps.extend_from_slice(deps);
        self.deps[from..].sort_unstable_by(|a, b| b.cmp(a));
        self.dep_at.push(self.deps.len());
        self.row.push(self.row[i] + i.div_ceil(64));
    }

    pub fn contains(&self, i: usize, j: usize) -> bool {
        if j >= i {
            return false;
        }
        if self.built.get() <= i {
            self.build_through(i);
        }
        self.bits.borrow()[self.row[i] + j / 64] & (1 << (j % 64)) != 0
    }

    /// Neither op is in the other's causal past.
    pub fn concurrent(&self, i: usize, j: usize) -> bool {
        i != j && !self.contains(i, j) && !self.contains(j, i)
    }

    /// Builds rows up to and including `last`. Each row is the union of
    /// its deps and their rows; a dep already in the row brings nothing
    /// new (its past is in the row with it), so the latest dep goes first
    /// and the rest are mostly skipped.
    fn build_through(&self, last: usize) {
        let mut bits = self.bits.borrow_mut();
        for i in self.built.get()..=last {
            bits.resize(self.row[i + 1], 0);
            let (head, row) = bits.split_at_mut(self.row[i]);
            for &d in &self.deps[self.dep_at[i]..self.dep_at[i + 1]] {
                if row[d / 64] & (1 << (d % 64)) == 0 {
                    for (t, f) in row.iter_mut().zip(&head[self.row[d]..self.row[d + 1]]) {
                        *t |= *f;
                    }
                    row[d / 64] |= 1 << (d % 64);
                }
            }
        }
        self.built.set(last + 1);
    }
}

/// An op hash as the key of `prepare`'s dep index, hashed by its first
/// eight bytes: the keys stored are hashes this reader computed (BLAKE3
/// over the op), so no writer can choose them, and the keyed SipHash of
/// eight bytes costs less than of thirty-two. Equality is the whole hash.
#[derive(PartialEq, Eq)]
struct IndexKey(OpHash);

impl Hash for IndexKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut lead = [0; 8];
        lead.copy_from_slice(&self.0 .0[..8]);
        state.write_u64(u64::from_le_bytes(lead));
    }
}

pub(super) fn prepare(ops: &[VerifiedOp]) -> OpSet<'_> {
    let mut sorted: Vec<&VerifiedOp> = ops.iter().collect();
    sorted.sort_by_key(|op| op.stamp());
    sorted.dedup_by_key(|op| op.hash());

    // Equivocation: two ops at one (author, seq). Every op of that author
    // from the fork on is dropped, on every reader alike.
    // Sorted by (seq, author, hash), two ops at one (author, seq) with
    // different hashes sit next to each other; seq leads the key because
    // it is a cheap compare that settles most pairs.
    let mut by_seq: Vec<(u64, &[u8; 32], OpHash)> = sorted
        .iter()
        .map(|op| (op.op().seq, &op.author().0, op.hash()))
        .collect();
    by_seq.sort_unstable();
    let mut fork_at: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    for pair in by_seq.windows(2) {
        let ((seq, author, a), (next_seq, next_author, b)) = (pair[0], pair[1]);
        if author == next_author && seq == next_seq && a != b {
            let slot = fork_at.entry(*author).or_insert(u64::MAX);
            *slot = (*slot).min(seq);
        }
    }

    let mut index: HashMap<IndexKey, usize> = HashMap::with_capacity(sorted.len());
    let mut kept: Vec<&VerifiedOp> = Vec::with_capacity(sorted.len());
    let mut past = Ancestry::with_capacity(sorted.len());
    let mut deps: Vec<usize> = Vec::new();
    let mut pending = Vec::new();
    for op in sorted {
        // An out-of-bounds op stays in the DAG (validation refuses it), so
        // the ops that cite it are not held pending forever.
        if fork_at
            .get(&op.author().0)
            .is_some_and(|seq| op.op().seq >= *seq)
        {
            continue;
        }
        deps.clear();
        let complete = op.op().deps.iter().all(|d| {
            index
                .get(&IndexKey(d.op_hash))
                .map(|&i| deps.push(i))
                .is_some()
        });
        if !complete {
            pending.push(op.hash());
            continue;
        }
        index.insert(IndexKey(op.hash()), kept.len());
        kept.push(op);
        past.push(&deps);
    }
    OpSet {
        ops: kept,
        past,
        pending,
        equivocators: fork_at.into_keys().map(PseudonymKey).collect(),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rekindle_types::id::PseudonymKey;

    use super::prepare;
    use crate::fixture::{meta, op};
    use crate::op::VerifiedOp;

    /// A DAG of `n` ops, op `k` citing the earlier ops `picks` selects.
    fn dag(n: usize, picks: &[u64]) -> Vec<VerifiedOp> {
        let mut ops: Vec<VerifiedOp> = Vec::new();
        for k in 0..n {
            let bits = picks[k % picks.len()].rotate_left(u32::try_from(k % 64).unwrap_or(0));
            let deps: Vec<&VerifiedOp> = (0..k)
                .filter(|&d| bits & (1 << ((k + 3 * d) % 64)) != 0 && k - d <= 70)
                .map(|d| &ops[d])
                .collect();
            let author = PseudonymKey([u8::try_from(k % 251).unwrap_or(0); 32]);
            let next = op(
                &author,
                u64::try_from(k).unwrap_or(0),
                &deps,
                k as u64,
                meta("m"),
            );
            ops.push(next);
        }
        ops
    }

    proptest! {
        /// R4-5: rows built on demand, in any query order, with the
        /// contained-dep skip, equal causal reachability over the deps.
        #[test]
        fn ancestry_is_reachability_in_any_query_order(
            n in 1usize..160,
            picks in proptest::collection::vec(any::<u64>(), 1..8),
            order in proptest::collection::vec(any::<u32>(), 1..400),
        ) {
            let ops = dag(n, &picks);
            let set = prepare(&ops);
            let at = |h| set.ops.iter().position(|o| o.hash() == h);
            let mut reach = vec![vec![false; set.ops.len()]; set.ops.len()];
            for i in 0..set.ops.len() {
                for d in &set.ops[i].op().deps {
                    let d = at(d.op_hash).unwrap_or(usize::MAX);
                    prop_assert!(d < i);
                    reach[i][d] = true;
                    let below = reach[d].clone();
                    for (r, b) in reach[i].iter_mut().zip(below) {
                        *r |= b;
                    }
                }
            }
            let m = set.ops.len();
            for q in order {
                let q = q as usize;
                let (i, j) = (q % m, (q / m) % m);
                prop_assert_eq!(set.past.contains(i, j), reach[i][j], "contains({}, {})", i, j);
            }
        }
    }
}
