//! Usable checkpoint hints (plan E3.2 S7c.4): which (height, round,
//! digest) a strict majority of the eligible members endorsed, which
//! endorsements count for nothing, and the equivocation that disables
//! every hint. A hint is acceleration, never truncation: no op's fate
//! depends on one, so rounds have no finality, a late majority in a lower
//! round just becomes the hint selected from then on, and nothing is
//! undone.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rekindle_types::governance::{Checkpoint, GenesisCert, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::cut::causal_past;
use super::electorate::{eligible_set, is_strict_majority, tally};
use super::shape::checkpoint_digest;
use crate::merge::merge_cut;
use crate::op::VerifiedOp;

/// Why a `Checkpoint` op counts for no digest; it is quarantined, shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndorsementFault {
    /// "two checkpoint endorsements in one round": its author signed
    /// another `Checkpoint` op at the same (height, round). Both count for
    /// neither digest (ADR 0018's write-once vote).
    TwoInOneRound,
    /// "checkpoint equivocation": it carries one of two digests that each
    /// have a strict majority for one cut and one `prevCheckpoint`.
    Equivocation,
}

/// A (height, round, digest) that a strict majority of the eligible
/// members' `Checkpoint` ops carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsableHint {
    pub height: u64,
    pub round: u32,
    pub digest: [u8; 32],
    /// The body, as the lowest `(lamport, op_hash)` op carrying it in this
    /// round has it.
    pub checkpoint: Checkpoint,
    /// That op.
    pub op: OpHash,
    /// The members whose ops in this round carry it, by key bytes.
    pub endorsers: BTreeSet<[u8; 32]>,
    /// The size of the electorate it was counted against.
    pub electorate: usize,
}

/// Provable equivocation: two digests (or more), each with a strict
/// majority, for one cut and one `prevCheckpoint`. The body is a pure
/// function of those two, so the conflicting ops are the proof every
/// reader derives from the op set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Equivocation {
    /// The conflicting digests, in byte order.
    pub digests: Vec<[u8; 32]>,
    /// The members whose ops carry more than one of them.
    pub flagged: Vec<PseudonymKey>,
}

/// Every hint an op set carries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hints {
    /// In `(height, round, digest)` order.
    pub usable: Vec<UsableHint>,
    pub equivocations: Vec<Equivocation>,
    /// The `Checkpoint` ops that count for nothing, in hash order.
    pub faults: Vec<(OpHash, EndorsementFault)>,
}

impl Hints {
    /// The hint selected at `height`: the lowest round with a majority,
    /// ties by digest bytes. None for any height once the community holds
    /// an equivocation: readers then fall back to full replay (hypercore's
    /// fail-closed freeze).
    #[must_use]
    pub fn selected(&self, height: u64) -> Option<&UsableHint> {
        if !self.equivocations.is_empty() {
            return None;
        }
        self.usable.iter().find(|h| h.height == height)
    }

    /// The selected hint at the greatest height that has one.
    #[must_use]
    pub fn latest(&self) -> Option<&UsableHint> {
        let height = self.usable.iter().map(|h| h.height).max()?;
        self.selected(height)
    }

    /// Whether the (height, round, digest) is a usable hint.
    #[must_use]
    pub fn is_usable(&self, height: u64, round: u32, digest: &[u8; 32]) -> bool {
        (self.usable.iter()).any(|h| h.height == height && h.round == round && h.digest == *digest)
    }
}

struct Endorsement<'a> {
    op: &'a VerifiedOp,
    body: &'a Checkpoint,
    digest: [u8; 32],
}

/// The hints `ops` carry, counting only the `Checkpoint` ops `counted`
/// admits (the ones the merge applied). The electorate of a height-h + 1
/// digest is the eligible set at its `prevCheckpoint`'s cut (the cert's
/// creator set at height 1), so a timeout never changes it and never
/// lowers the majority.
#[must_use]
pub fn hints(cert: &GenesisCert, ops: &[VerifiedOp], counted: impl Fn(&OpHash) -> bool) -> Hints {
    let all: Vec<Endorsement<'_>> = (ops.iter())
        .filter(|o| counted(&o.hash()))
        .filter_map(|op| match op.body() {
            GovernanceEntry::Checkpoint(body) => Some(Endorsement {
                op,
                body,
                digest: checkpoint_digest(body),
            }),
            _ => None,
        })
        .collect();
    let mut faults: BTreeMap<OpHash, EndorsementFault> = BTreeMap::new();
    let mut per_round: BTreeMap<([u8; 32], u64, u32), Vec<OpHash>> = BTreeMap::new();
    for e in &all {
        let key = (e.op.author().0, e.body.height, e.body.round);
        per_round.entry(key).or_default().push(e.op.hash());
    }
    for hashes in per_round.values().filter(|h| h.len() > 1) {
        for hash in hashes {
            faults.insert(*hash, EndorsementFault::TwoInOneRound);
        }
    }
    let usable = count(cert, ops, &all, &faults);
    let equivocations = equivocations(&all, &usable);
    for e in &all {
        if equivocations.iter().any(|q| q.digests.contains(&e.digest)) {
            faults
                .entry(e.op.hash())
                .or_insert(EndorsementFault::Equivocation);
        }
    }
    Hints {
        usable,
        equivocations,
        faults: faults.into_iter().collect(),
    }
}

/// The usable hints: per (height, `prevCheckpoint` digest, round), the
/// digests a strict majority of that electorate carries, from the ops not
/// faulted.
fn count(
    cert: &GenesisCert,
    ops: &[VerifiedOp],
    all: &[Endorsement<'_>],
    faults: &BTreeMap<OpHash, EndorsementFault>,
) -> Vec<UsableHint> {
    let by_hash: HashMap<OpHash, &Endorsement<'_>> = all.iter().map(|e| (e.op.hash(), e)).collect();
    type Group<'a, 'b> = Vec<&'b Endorsement<'a>>;
    let mut groups: BTreeMap<(u64, Option<[u8; 32]>, u32), Group<'_, '_>> = BTreeMap::new();
    for e in all.iter().filter(|e| !faults.contains_key(&e.op.hash())) {
        let prev = match &e.body.prev {
            None => None,
            Some(r) => match by_hash.get(&r.op_hash) {
                Some(p) => Some(p.digest),
                None => continue,
            },
        };
        groups
            .entry((e.body.height, prev, e.body.round))
            .or_default()
            .push(e);
    }
    let mut electorates: HashMap<Option<[u8; 32]>, Vec<PseudonymKey>> = HashMap::new();
    let mut usable = Vec::new();
    for ((height, prev, round), group) in groups {
        let electorate = electorates.entry(prev).or_insert_with(|| {
            let prev_body = prev
                .and_then(|d| all.iter().find(|e| e.digest == d))
                .map(|e| e.body);
            let state = prev_body.map(|b| merge_cut(cert, &causal_past(ops, &b.cut)).0);
            eligible_set(cert, state.as_ref())
        });
        let votes = tally(electorate, group.iter().map(|e| (e.op.author(), e.body)));
        for (digest, endorsers) in votes {
            if !is_strict_majority(endorsers.len(), electorate.len()) {
                continue;
            }
            let Some(first) = (group.iter())
                .filter(|e| e.digest == digest)
                .min_by_key(|e| (e.op.lamport(), e.op.hash()))
            else {
                continue;
            };
            usable.push(UsableHint {
                height,
                round,
                digest,
                checkpoint: first.body.clone(),
                op: first.op.hash(),
                endorsers,
                electorate: electorate.len(),
            });
        }
    }
    usable.sort_by(|a, b| (a.height, a.round, a.digest).cmp(&(b.height, b.round, b.digest)));
    usable
}

/// Two digests with a strict majority for one cut (its heads, sorted) and
/// one `prevCheckpoint` (by the hash of the op it names). Two different
/// cuts at one height are not equivocation: both are valid hints.
fn equivocations(all: &[Endorsement<'_>], usable: &[UsableHint]) -> Vec<Equivocation> {
    let mut by_cut: BTreeMap<(Vec<OpHash>, Option<OpHash>), BTreeSet<[u8; 32]>> = BTreeMap::new();
    for hint in usable {
        let mut heads: Vec<OpHash> = hint.checkpoint.cut.iter().map(|r| r.op_hash).collect();
        heads.sort_unstable();
        let prev = hint.checkpoint.prev.as_ref().map(|r| r.op_hash);
        by_cut.entry((heads, prev)).or_default().insert(hint.digest);
    }
    by_cut
        .into_values()
        .filter(|digests| digests.len() > 1)
        .map(|digests| {
            let mut carried: BTreeMap<[u8; 32], BTreeSet<[u8; 32]>> = BTreeMap::new();
            for e in all.iter().filter(|e| digests.contains(&e.digest)) {
                carried.entry(e.op.author().0).or_default().insert(e.digest);
            }
            Equivocation {
                digests: digests.into_iter().collect(),
                flagged: (carried.into_iter())
                    .filter(|(_, d)| d.len() > 1)
                    .map(|(author, _)| PseudonymKey(author))
                    .collect(),
            }
        })
        .collect()
}
