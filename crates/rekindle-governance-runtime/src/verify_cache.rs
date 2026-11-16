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

/// Whether every op `op` names by hash is verified in `known`: its deps,
/// and its `prev` at seq > 0 (the ones `verify` looks up).
fn ready(op: &GovernanceOp, known: &HashMap<OpHash, VerifiedOp>) -> bool {
    let deps = op.deps.iter().all(|d| known.contains_key(&d.op_hash));
    let prev = match (op.seq, op.prev) {
        (seq, Some(prev)) if seq > 0 => known.contains_key(&prev),
        _ => true,
    };
    deps && prev
}

/// What one pass decides for a waiting op.
enum Step {
    Verified(Box<VerifiedOp>),
    Waiting,
    Refused,
}

/// Decode and verify `held` for community `anchor` in dependency order,
/// reusing `cache`'s verdicts and keeping the new ones; the cache then holds
/// the verdicts of `held`'s ops only.
#[must_use]
pub fn verify_held_cached(anchor: &str, held: &[HeldOp], cache: &OpVerifyCache) -> VerifiedSet {
    let mut set = VerifiedSet::default();
    let mut waiting: Vec<(OpHash, GovernanceOp, Vec<u8>)> = Vec::new();
    for op in held {
        match decode_op(&op.encoded) {
            Ok(decoded) if op_hash(&decoded.unsigned, &decoded.op.sig) == op.hash => {
                waiting.push((op.hash, decoded.op, decoded.unsigned));
            }
            _ => set.quarantined.push(op.hash),
        }
    }
    let kept: HashMap<OpHash, Verdict> = {
        let inner = cache.inner.lock();
        waiting
            .iter()
            .filter_map(|(h, _, _)| inner.verdicts.get(h).map(|v| (*h, v.clone())))
            .collect()
    };
    let mut fresh: HashMap<OpHash, Verdict> = HashMap::new();
    let mut verifications = 0u64;
    let mut known: HashMap<OpHash, VerifiedOp> = HashMap::new();
    // This read's ops whose signature and proofs passed and which wait on a
    // dep (`verify` checks those first): not a verdict, never kept.
    let mut authentic: HashSet<OpHash> = HashSet::new();
    loop {
        let mut progressed = Vec::new();
        let mut still = Vec::new();
        for (hash, op, unsigned) in waiting {
            let step = match kept.get(&hash) {
                Some(Verdict::Refused(
                    OpRejection::BadSignature
                    | OpRejection::BadInviteProof
                    | OpRejection::BadOwnerProof,
                )) => Step::Refused,
                Some(Verdict::Refused(_)) if ready(&op, &known) => Step::Refused,
                Some(Verdict::Verified(v)) if ready(&op, &known) => {
                    Step::Verified(Box::new(VerifiedOp::clone(v)))
                }
                Some(_) => Step::Waiting,
                // Authentic and still short of a dep: `verify` would say
                // `MissingDep` again after one more signature check.
                None if authentic.contains(&hash) && !ready(&op, &known) => Step::Waiting,
                None => {
                    verifications += 1;
                    match VerifiedOp::verify(anchor, op.clone(), &unsigned, |h| known.get(h)) {
                        Ok(verified) => {
                            fresh.insert(hash, Verdict::Verified(Arc::new(verified.clone())));
                            Step::Verified(Box::new(verified))
                        }
                        Err(OpRejection::MissingDep(_)) => {
                            authentic.insert(hash);
                            Step::Waiting
                        }
                        Err(rejection) => {
                            fresh.insert(hash, Verdict::Refused(rejection));
                            Step::Refused
                        }
                    }
                }
            };
            match step {
                Step::Verified(verified) => progressed.push(*verified),
                Step::Waiting => still.push((hash, op, unsigned)),
                Step::Refused => set.quarantined.push(hash),
            }
        }
        waiting = still;
        if progressed.is_empty() {
            break;
        }
        for verified in progressed {
            set.ops.push(verified.clone());
            known.insert(verified.hash(), verified);
        }
    }
    {
        let held_hashes: HashSet<OpHash> = held.iter().map(|op| op.hash).collect();
        let mut inner = cache.inner.lock();
        inner.verdicts.extend(fresh);
        inner.verdicts.retain(|h, _| held_hashes.contains(h));
        inner.verifications += verifications;
    }
    let waiting_hashes: BTreeSet<OpHash> = waiting.iter().map(|(h, _, _)| *h).collect();
    let mut missing: BTreeMap<OpHash, OpRef> = BTreeMap::new();
    for (hash, op, _) in &waiting {
        set.pending.push(*hash);
        for dep in &op.deps {
            if !known.contains_key(&dep.op_hash) && !waiting_hashes.contains(&dep.op_hash) {
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
