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
pub(super) struct Ancestry {
    words: usize,
    bits: Vec<u64>,
}

impl Ancestry {
    fn new(n: usize) -> Self {
        let words = n.div_ceil(64).max(1);
        Self {
            words,
            bits: vec![0; words * n],
        }
    }

    pub fn contains(&self, i: usize, j: usize) -> bool {
        self.bits[i * self.words + j / 64] & (1 << (j % 64)) != 0
    }

    /// Neither op is in the other's causal past.
    pub fn concurrent(&self, i: usize, j: usize) -> bool {
        i != j && !self.contains(i, j) && !self.contains(j, i)
    }

    fn inherit(&mut self, i: usize, d: usize) {
        let w = self.words;
        for k in 0..w {
            let v = self.bits[d * w + k];
            self.bits[i * w + k] |= v;
        }
        self.bits[i * w + d / 64] |= 1 << (d % 64);
    }
}

pub(super) fn prepare(ops: &[VerifiedOp]) -> OpSet<'_> {
    let mut sorted: Vec<&VerifiedOp> = ops.iter().collect();
    sorted.sort_by_key(|op| op.stamp());
    sorted.dedup_by_key(|op| op.hash());

    // Equivocation: two ops at one (author, seq). Every op of that author
    // from the fork on is dropped, on every reader alike.
    let mut at_seq: BTreeMap<(&[u8; 32], u64), OpHash> = BTreeMap::new();
    let mut fork_at: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    for op in &sorted {
        let key = (&op.author().0, op.op().seq);
        if let Some(other) = at_seq.insert(key, op.hash()) {
            if other != op.hash() {
                let slot = fork_at.entry(op.author().0).or_insert(u64::MAX);
                *slot = (*slot).min(op.op().seq);
            }
        }
    }

    let mut index: HashMap<OpHash, usize> = HashMap::new();
    let mut kept: Vec<&VerifiedOp> = Vec::new();
    let mut deps_of: Vec<Vec<usize>> = Vec::new();
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
