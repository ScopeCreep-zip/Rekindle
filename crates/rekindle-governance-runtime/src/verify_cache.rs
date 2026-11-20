//! One community's op verdicts, kept for the process (plan E3.2 S7, R4-5).
//!
//! [`VerifiedOp::verify`] is a pure function of the anchor, the op's
//! canonical unsigned bytes and signature, and its deps' verified forms:
//! the author signature and the invite and owner proofs read only the
//! anchor and the op, and the dep, Lamport and chain checks read only the
//! ops its `OpRef`s and `prev` name by hash. An [`OpHash`] commits to the
//! unsigned bytes and the signature ([`op_hash`]), so one verdict per hash
//! holds for the life of the process. Every read of the op set (a rebuild,
//! the sweep's convergence check, a gossiped op, the bootstrap reply) then
//! verifies only the ops it has not seen.
//!
//! - **Verified** ops are kept with their validated Lamport value, and are
//!   reused once every dep (and `prev`, at seq > 0) is verified in the same
//!   read: exactly when `verify` would succeed again.
//! - **Refused** ops are kept with their rejection. A signature or proof
//!   rejection needs nothing else; a `DepMismatch` or `BadChain` is reused
//!   under the same readiness as a verified op, since `verify` reaches it
//!   only after finding every dep.
//! - `MissingDep` is never kept: it changes when the dep arrives. Within
//!   one read, an op that got it is verified again only once its deps are,
//!   not on every pass.
//!
//! The cache holds only the ops of the last held set it was read with, so it
//! is bounded by the community's op store; the Host drops it with the
//! community (leave, session end).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use rekindle_codec::governance_op::decode_op;
use rekindle_governance::op::{op_hash, OpRejection, VerifiedOp};
use rekindle_types::governance::{GovernanceOp, OpHash, OpRef};

use crate::op_log::{HeldOp, VerifiedSet};

/// One op's kept verdict.
#[derive(Debug, Clone)]
enum Verdict {
    Verified(Arc<VerifiedOp>),
    Refused(OpRejection),
}

#[derive(Debug, Default)]
struct Inner {
    verdicts: HashMap<OpHash, Verdict>,
    /// How many times `VerifiedOp::verify` ran through this cache.
    verifications: u64,
    /// The reads' units of work ([`OpVerifyCache::walk_steps`]).
    walk_steps: u64,
}

/// One community's op verdicts by hash. Owned per community by the Host
/// (`GovernanceRuntimeDeps::op_verify_cache`).
#[derive(Debug, Default)]
pub struct OpVerifyCache {
    inner: parking_lot::Mutex<Inner>,
}

impl OpVerifyCache {
    /// How many signature verifications (`VerifiedOp::verify` calls) reads
    /// through this cache have run.
    #[must_use]
    pub fn verifications(&self) -> u64 {
        self.inner.lock().verifications
    }

    /// The units of work reads through this cache have done: one per held
    /// op decoded, per readiness check of an op in a pass, and per
    /// dependent visited when an op is verified. A read's cost in the
    /// machine-independent unit its tests assert growth in
    /// (`tests/verify_held_cost.rs`): linear in the ops and their deps.
    #[must_use]
    pub fn walk_steps(&self) -> u64 {
        self.inner.lock().walk_steps
    }

    /// How many verdicts are kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().verdicts.len()
    }

    /// Whether no verdict is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().verdicts.is_empty()
    }
}

/// The ops `op` names by hash and `verify` looks up, distinct: its deps,
/// and its `prev` at seq > 0. It is ready once every one is verified in
/// the read.
fn waits_on(op: &GovernanceOp) -> HashSet<OpHash> {
    let prev = match (op.seq, op.prev) {
        (seq, Some(prev)) if seq > 0 => Some(prev),
        _ => None,
    };
    op.deps.iter().map(|d| d.op_hash).chain(prev).collect()
}

/// What one pass decides for a waiting op.
enum Step {
    Verified(Box<VerifiedOp>),
    Waiting,
    Refused,
}

/// One read's walk state: the decoded ops in held order, the verified ones
/// by hash, and for each op how many of the hashes it waits on are not yet
/// verified.
struct Walk<'a> {
    anchor: &'a str,
    entries: Vec<(OpHash, GovernanceOp, Vec<u8>)>,
    kept: HashMap<OpHash, Verdict>,
    fresh: HashMap<OpHash, Verdict>,
    verifications: u64,
    /// This read's units of work, added to the cache's `walk_steps`.
    steps: u64,
    known: HashMap<OpHash, VerifiedOp>,
    /// This read's ops whose signature and proofs passed and which wait on
    /// a dep (`verify` checks those first): not a verdict, never kept.
    authentic: HashSet<OpHash>,
    unmet: Vec<usize>,
}

impl Walk<'_> {
    /// The step for entry `i` in this pass; `ready` exactly when every hash
    /// it waits on was verified in an earlier pass.
    fn step(&mut self, i: usize) -> Step {
        self.steps += 1;
        let ready = self.unmet[i] == 0;
        let (hash, op, unsigned) = &self.entries[i];
        match self.kept.get(hash) {
            Some(Verdict::Refused(
                OpRejection::BadSignature
                | OpRejection::BadInviteProof
                | OpRejection::BadOwnerProof,
            )) => Step::Refused,
            Some(Verdict::Refused(_)) if ready => Step::Refused,
            Some(Verdict::Verified(v)) if ready => Step::Verified(Box::new(VerifiedOp::clone(v))),
            Some(_) => Step::Waiting,
            // Authentic and still short of a dep: `verify` would say
            // `MissingDep` again after one more signature check.
            None if self.authentic.contains(hash) && !ready => Step::Waiting,
            None => {
                self.verifications += 1;
                let known = &self.known;
                match VerifiedOp::verify(self.anchor, op.clone(), unsigned, |h| known.get(h)) {
                    Ok(verified) => {
                        let kept = Verdict::Verified(Arc::new(verified.clone()));
                        self.fresh.insert(*hash, kept);
                        Step::Verified(Box::new(verified))
                    }
                    Err(OpRejection::MissingDep(_)) => {
                        self.authentic.insert(*hash);
                        Step::Waiting
                    }
                    Err(rejection) => {
                        self.fresh.insert(*hash, Verdict::Refused(rejection));
                        Step::Refused
                    }
                }
            }
        }
    }
}

/// Decode and verify `held` for community `anchor` in dependency order,
/// reusing `cache`'s verdicts and keeping the new ones; the cache then holds
/// the verdicts of `held`'s ops only.
///
/// The walk runs in passes: an op is decided in the first pass after every
/// op it waits on was verified (all ops in the first), so the verified ops
/// come out level by level, in held order within a level. Each pass visits
/// only the ops the previous one made ready (a ready queue, Kahn's
/// algorithm): linear in the ops and their deps.
#[must_use]
pub fn verify_held_cached(anchor: &str, held: &[HeldOp], cache: &OpVerifyCache) -> VerifiedSet {
    let mut set = VerifiedSet::default();
    let mut entries: Vec<(OpHash, GovernanceOp, Vec<u8>)> = Vec::new();
    let mut decodes: u64 = 0;
    for op in held {
        decodes += 1;
        match decode_op(&op.encoded) {
            Ok(decoded) if op_hash(&decoded.unsigned, &decoded.op.sig) == op.hash => {
                entries.push((op.hash, decoded.op, decoded.unsigned));
            }
            _ => set.quarantined.push(op.hash),
        }
    }
    let kept: HashMap<OpHash, Verdict> = {
        let inner = cache.inner.lock();
        entries
            .iter()
            .filter_map(|(h, _, _)| inner.verdicts.get(h).map(|v| (*h, v.clone())))
            .collect()
    };
    // Who waits on each hash, by entry index.
    let mut dependents: HashMap<OpHash, Vec<usize>> = HashMap::new();
    let mut unmet = Vec::with_capacity(entries.len());
    for (i, (_, op, _)) in entries.iter().enumerate() {
        let names = waits_on(op);
        unmet.push(names.len());
        for name in names {
            dependents.entry(name).or_default().push(i);
        }
    }
    let mut walk = Walk {
        anchor,
        entries,
        kept,
        fresh: HashMap::new(),
        verifications: 0,
        steps: decodes,
        known: HashMap::new(),
        authentic: HashSet::new(),
        unmet,
    };
    let mut decided = vec![false; walk.entries.len()];
    let mut pass: Vec<usize> = (0..walk.entries.len()).collect();
    loop {
        let mut progressed = Vec::new();
        for &i in &pass {
            match walk.step(i) {
                Step::Verified(verified) => {
                    decided[i] = true;
                    progressed.push(*verified);
                }
                Step::Waiting => {}
                Step::Refused => {
                    decided[i] = true;
                    set.quarantined.push(walk.entries[i].0);
                }
            }
        }
        if progressed.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for verified in progressed {
            set.ops.push(verified.clone());
            let hash = verified.hash();
            if walk.known.insert(hash, verified).is_none() {
                for &d in dependents.get(&hash).into_iter().flatten() {
                    walk.steps += 1;
                    walk.unmet[d] -= 1;
                    if walk.unmet[d] == 0 && !decided[d] {
                        next.push(d);
                    }
                }
            }
        }
        next.sort_unstable();
        next.dedup();
        pass = next;
    }
    {
        let held_hashes: HashSet<OpHash> = held.iter().map(|op| op.hash).collect();
        let mut inner = cache.inner.lock();
        inner.verdicts.extend(walk.fresh);
        inner.verdicts.retain(|h, _| held_hashes.contains(h));
        inner.verifications += walk.verifications;
        inner.walk_steps += walk.steps;
    }
    let waiting: Vec<&(OpHash, GovernanceOp, Vec<u8>)> = (walk.entries.iter())
        .zip(&decided)
        .filter_map(|(entry, decided)| (!decided).then_some(entry))
        .collect();
    let waiting_hashes: BTreeSet<OpHash> = waiting.iter().map(|(h, _, _)| *h).collect();
    let mut missing: BTreeMap<OpHash, OpRef> = BTreeMap::new();
    for (hash, op, _) in waiting {
        set.pending.push(*hash);
        for dep in &op.deps {
            if !walk.known.contains_key(&dep.op_hash) && !waiting_hashes.contains(&dep.op_hash) {
                missing.entry(dep.op_hash).or_insert_with(|| dep.clone());
            }
        }
    }
    set.missing = missing.into_values().collect();
    set
}

#[cfg(test)]
#[path = "verify_cache_tests.rs"]
mod tests;
