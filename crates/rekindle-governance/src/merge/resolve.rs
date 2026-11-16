//! Op-set preparation for `merge`: dedup, equivocation, causal completeness
//! and the ancestry every resolution rule asks about (plan E3.2).

use std::collections::{BTreeMap, HashMap};

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
/// (a lower-triangular bit matrix, half the memory and work of a square one).
pub(super) struct Ancestry {
    /// Where row `i`'s words start in `bits`.
    row: Vec<usize>,
    bits: Vec<u64>,
}

impl Ancestry {
    fn new(n: usize) -> Self {
        let mut row = Vec::with_capacity(n + 1);
        let mut at = 0;
        for i in 0..n {
            row.push(at);
            at += i.div_ceil(64);
        }
        row.push(at);
        Self {
            row,
            bits: vec![0; at],
        }
    }

    pub fn contains(&self, i: usize, j: usize) -> bool {
        j < i && self.bits[self.row[i] + j / 64] & (1 << (j % 64)) != 0
    }

    /// Neither op is in the other's causal past.
    pub fn concurrent(&self, i: usize, j: usize) -> bool {
        i != j && !self.contains(i, j) && !self.contains(j, i)
    }

    /// Op `d` (a dep of `i`, so `d < i`) and its past join `i`'s past.
    fn inherit(&mut self, i: usize, d: usize) {
        debug_assert!(d < i, "a dep precedes its dependant in stamp order");
        let (head, tail) = self.bits.split_at_mut(self.row[i]);
        let from = &head[self.row[d]..self.row[d + 1]];
        let to = &mut tail[..self.row[i + 1] - self.row[i]];
        for (t, f) in to.iter_mut().zip(from) {
            *t |= *f;
        }
        to[d / 64] |= 1 << (d % 64);
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

    let mut index: HashMap<OpHash, usize> = HashMap::with_capacity(sorted.len());
    let mut kept: Vec<&VerifiedOp> = Vec::with_capacity(sorted.len());
    let mut deps_of: Vec<Vec<usize>> = Vec::with_capacity(sorted.len());
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
        let deps: Option<Vec<usize>> = op
            .op()
            .deps
            .iter()
            .map(|d| index.get(&d.op_hash).copied())
            .collect();
        let Some(deps) = deps else {
            pending.push(op.hash());
            continue;
        };
        index.insert(op.hash(), kept.len());
        kept.push(op);
        deps_of.push(deps);
    }

    let mut past = Ancestry::new(kept.len());
    for (i, deps) in deps_of.iter().enumerate() {
        for &d in deps {
            past.inherit(i, d);
        }
    }
    OpSet {
        ops: kept,
        past,
        pending,
        equivocators: fork_at.into_keys().map(PseudonymKey).collect(),
    }
}
