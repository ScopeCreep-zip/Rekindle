//! The governance op log (plan E3.2, ADR 0011): our signed ops are built,
//! verified, stored and published here, and the community's state is
//! re-merged from the held op set under the pinned `GenesisCert`.
//!
//! - **Append** ([`append_op`]): author = our pseudonym, seq = our next
//!   seq, prev = our last op, deps = the current frontier (≤ 64 `OpRef`s),
//!   `issued_at` from the clock port, signed by our community pseudonym
//!   over `GOV_OP ‖ anchor ‖ op_unsigned_bytes`. The op is verified as any
//!   reader verifies it, stored in `governance_ops`, then queued on the
//!   governance-op outbox stream, which appends it to our member record's
//!   op log (`M`/2, `rekindle_records::chain::append_at`).
//! - **Rebuild** ([`rebuild_ops`], [`rebuild_at`]): the held ops are
//!   decoded and verified in dependency order; a missing dep is fetched by
//!   its `OpRef`; the verified set merges under the cert.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rekindle_codec::governance_op::{decode_op, encode_op, op_unsigned_bytes};
use rekindle_governance::merge::{self, MergeOutcome};
use rekindle_governance::op::{op_hash, op_signing_bytes, OpRejection, VerifiedOp};
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;
use rekindle_types::governance::{GenesisCert, GovernanceEntry, GovernanceOp, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// An op's fate in the last merge, as `governance_ops.state` stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpFate {
    Applied,
    PendingDeps,
    Quarantined,
    Equivocation,
}

/// One op as the op store holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldOp {
    pub hash: OpHash,
    pub author: PseudonymKey,
    /// The author's member record (the chain the op lives in).
    pub author_record: String,
    pub seq: u64,
    pub lamport: u64,
    pub issued_at: u64,
    /// `rekindle_codec::governance_op::encode_op`.
    pub encoded: Vec<u8>,
    pub fate: OpFate,
}

/// The held op set, verified.
#[derive(Debug, Default)]
pub struct VerifiedSet {
    /// Verified ops in dependency order.
    pub ops: Vec<VerifiedOp>,
    /// Ops still waiting on a dep.
    pub pending: Vec<OpHash>,
    /// Undecodable or refused ops.
    pub quarantined: Vec<OpHash>,
    /// Deps cited by a held op and not held: fetched by their locator.
    pub missing: Vec<OpRef>,
}

/// Decode and verify `held` for community `anchor` in dependency order.
#[must_use]
pub fn verify_held(anchor: &str, held: &[HeldOp]) -> VerifiedSet {
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
    let mut known: HashMap<OpHash, VerifiedOp> = HashMap::new();
    loop {
        let mut progressed = Vec::new();
        let mut still = Vec::new();
        for (hash, op, unsigned) in waiting {
            match VerifiedOp::verify(anchor, op.clone(), &unsigned, |h| known.get(h)) {
                Ok(verified) => progressed.push(verified),
                Err(OpRejection::MissingDep(_)) => still.push((hash, op, unsigned)),
                Err(_) => set.quarantined.push(hash),
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

/// The member record an author's ops live in: the cert's for the
/// creator, else the author's `MemberJoined` in the merged state.
#[must_use]
pub fn author_record(
    cert: &GenesisCert,
    state: &GovernanceState,
    author: &PseudonymKey,
) -> Option<String> {
    if *author == cert.creator_pseudonym {
        return Some(cert.creator_member_record.clone());
    }
    state
        .members
        .get(author)
        .map(|m| m.member_record.clone())
        .or_else(|| {
            state
                .pending_members
                .get(author)
                .map(|m| m.member_record.clone())
        })
}

/// The frontier: the verified ops no other verified op cites, newest
/// first by `(lamport, hash)`, at most [`MAX_FRONTIER_REFS`], always
/// holding `must` (our previous op, which the chain rule requires among
/// the deps). Returned in hash order, the canonical order on the wire.
fn frontier(ops: &[VerifiedOp], must: Option<OpHash>) -> Vec<&VerifiedOp> {
    let cited: BTreeSet<OpHash> = ops
        .iter()
        .flat_map(|op| op.op().deps.iter().map(|d| d.op_hash))
        .collect();
    let mut heads: Vec<&VerifiedOp> = ops
        .iter()
        .filter(|op| !cited.contains(&op.hash()))
        .collect();
    heads.sort_by_key(|op| std::cmp::Reverse(op.stamp()));
    heads.truncate(MAX_FRONTIER_REFS);
    if let Some(prev) = must {
        if !heads.iter().any(|op| op.hash() == prev) {
            if heads.len() == MAX_FRONTIER_REFS {
                heads.pop();
            }
            heads.extend(ops.iter().find(|op| op.hash() == prev));
        }
    }
    heads.sort_by_key(|op| op.hash());
    heads
}

/// Our op for `body`: the next seq of our chain, the frontier as deps,
/// signed by `key` (our community pseudonym) and verified against `set`
/// as every reader verifies it.
///
/// # Errors
/// A dep's author has no member record in `state`, the op does not
/// encode, or it fails verification.
pub fn build_op(
    anchor: &str,
    cert: &GenesisCert,
    state: &GovernanceState,
    set: &VerifiedSet,
    key: &SigningKey,
    body: GovernanceEntry,
    issued_at: u64,
) -> Result<(VerifiedOp, Vec<u8>), GovernanceRuntimeError> {
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let last = set
        .ops
        .iter()
        .filter(|op| *op.author() == author)
        .max_by_key(|op| op.op().seq);
    let (seq, prev) = last.map_or((0, None), |op| (op.op().seq + 1, Some(op.hash())));
    let deps = frontier(&set.ops, prev)
        .into_iter()
        .map(|op| {
            author_record(cert, state, op.author())
                .map(|record| op.op_ref(&record))
                .ok_or_else(|| {
                    GovernanceRuntimeError::MemberRecordMissing(hex::encode(op.author().0))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut op = GovernanceOp {
        author,
        seq,
        prev,
        deps,
        issued_at,
        body,
        sig: [0; 64],
    };
    let unsigned = op_unsigned_bytes(&op);
    op.sig = key.sign(&op_signing_bytes(anchor, &unsigned)).to_bytes();
    let encoded = encode_op(&op).map_err(|e| GovernanceRuntimeError::Encoding(e.to_string()))?;
    let known: HashMap<OpHash, &VerifiedOp> = set.ops.iter().map(|o| (o.hash(), o)).collect();
    let verified = VerifiedOp::verify(anchor, op, &unsigned, |h| known.get(h).copied())
        .map_err(|e| GovernanceRuntimeError::OpRefused(format!("{e:?}")))?;
    Ok((verified, encoded))
}

/// The stored form of a verified op.
#[must_use]
pub fn held(op: &VerifiedOp, author_record: String, encoded: Vec<u8>, fate: OpFate) -> HeldOp {
    HeldOp {
        hash: op.hash(),
        author: op.author().clone(),
        author_record,
        seq: op.op().seq,
        lamport: op.lamport(),
        issued_at: op.op().issued_at,
        encoded,
        fate,
    }
}

/// The fate of every held op after `outcome`.
#[must_use]
pub fn fates(set: &VerifiedSet, outcome: &MergeOutcome) -> Vec<(OpHash, OpFate)> {
    let equivocators = &outcome.equivocators;
    let mut out: Vec<(OpHash, OpFate)> = set
        .ops
        .iter()
        .map(|op| {
            let fate = if equivocators.contains(op.author()) {
                OpFate::Equivocation
            } else {
                OpFate::Applied
            };
            (op.hash(), fate)
        })
        .collect();
    out.extend(set.pending.iter().map(|h| (*h, OpFate::PendingDeps)));
    out.extend(set.quarantined.iter().map(|h| (*h, OpFate::Quarantined)));
    out
}

/// Re-read `community_id`'s op set from the op store, fetch every missing
/// dep by its `OpRef` (until a pass fetches nothing new), and merge under
/// the pinned cert. Records each op's fate. Returns the merged state.
///
/// # Errors
/// No cert is pinned, or the op store failed.
pub async fn rebuild_ops<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<GovernanceState, GovernanceRuntimeError> {
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let mut held = deps.governance_ops(community_id).await?;
    let mut tried: BTreeSet<OpHash> = BTreeSet::new();
    loop {
        let set = verify_held(community_id, &held);
        let wanted: Vec<OpRef> = set
            .missing
            .iter()
            .filter(|r| tried.insert(r.op_hash))
            .cloned()
            .collect();
        let mut fetched = false;
        for op_ref in wanted {
            let Some(op) = fetch_held(deps, &op_ref).await? else {
                continue;
            };
            deps.store_governance_op(community_id, &op).await?;
            held.push(op);
            fetched = true;
        }
        if !fetched {
            let outcome = merge::merge_outcome(&cert, &set.ops);
            deps.set_governance_op_fates(community_id, fates(&set, &outcome))
                .await?;
            return Ok(outcome.state);
        }
    }
}

/// The op `op_ref` names, fetched from its author's op log and checked
/// against the locator's hash; `None` when it is not there or does not
/// match.
///
/// # Errors
/// The fetch failed.
pub async fn fetch_held<D: GovernanceRuntimeDeps>(
    deps: &D,
    op_ref: &OpRef,
) -> Result<Option<HeldOp>, GovernanceRuntimeError> {
    let Some(encoded) = deps.fetch_governance_op(op_ref).await? else {
        return Ok(None);
    };
    let Ok(decoded) = decode_op(&encoded) else {
        return Ok(None);
    };
    if op_hash(&decoded.unsigned, &decoded.op.sig) != op_ref.op_hash {
        return Ok(None);
    }
    Ok(Some(HeldOp {
        hash: op_ref.op_hash,
        author: decoded.op.author.clone(),
        author_record: op_ref.author_record.clone(),
        seq: decoded.op.seq,
        lamport: 0,
        issued_at: decoded.op.issued_at,
        encoded,
        fate: OpFate::PendingDeps,
    }))
}

/// The gossip fast path (`governanceOp @74`): store an op a member
/// published, as a fetched one, when it is new, its author's member record
/// is known (its own `MemberJoined`, or the merged state's) and it does not
/// verify as refused; then rebuild, which fetches its missing deps by
/// `OpRef`. `true`: it was stored.
///
/// # Errors
/// No cert is pinned, or the op store failed.
pub async fn receive_op<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    encoded: Vec<u8>,
) -> Result<bool, GovernanceRuntimeError> {
    let Ok(decoded) = decode_op(&encoded) else {
        return Ok(false);
    };
    let hash = op_hash(&decoded.unsigned, &decoded.op.sig);
    let mut held = deps.governance_ops(community_id).await?;
    if held.iter().any(|op| op.hash == hash) {
        return Ok(false);
    }
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let record = match &decoded.op.body {
        rekindle_types::governance::GovernanceEntry::MemberJoined { member_record, .. }
            if decoded.op.seq == 0 =>
        {
            Some(member_record.clone())
        }
        _ => author_record(
            &cert,
            &deps.governance_state(community_id).unwrap_or_default(),
            &decoded.op.author,
        ),
    };
    let Some(author_record) = record else {
        return Ok(false);
    };
    let op = HeldOp {
        hash,
        author: decoded.op.author.clone(),
        author_record,
        seq: decoded.op.seq,
        lamport: 0,
        issued_at: decoded.op.issued_at,
        encoded,
        fate: OpFate::PendingDeps,
    };
    held.push(op.clone());
    if verify_held(community_id, &held).quarantined.contains(&hash) {
        return Ok(false);
    }
    deps.store_governance_op(community_id, &op).await?;
    crate::dht_hydration::rebuild_governance(deps, community_id).await;
    Ok(true)
}

/// Walk the op DAG of `anchor` from `start` (a member's `Frontier`) by
/// `OpRef`, through deps, until a pass fetches nothing new: the joiner's
/// read of the op set with nobody online. Nothing is stored; the caller
/// stores the walked ops once its community row exists.
///
/// # Errors
/// A fetch failed.
pub async fn walk<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    start: &[OpRef],
) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
    let mut held: Vec<HeldOp> = Vec::new();
    let mut tried: BTreeSet<OpHash> = BTreeSet::new();
    let mut wanted: Vec<OpRef> = start.to_vec();
    loop {
        let mut fetched = false;
        for op_ref in wanted.into_iter().filter(|r| tried.insert(r.op_hash)) {
            if let Some(op) = fetch_held(deps, &op_ref).await? {
                held.push(op);
                fetched = true;
            }
        }
        if !fetched {
            return Ok(held);
        }
        wanted = verify_held(anchor, &held).missing;
    }
}

/// Our `Frontier` (`M`/3): the heads of the verified set as `OpRef`s, in
/// hash order; a head whose author has no member record in `state` is
/// left out.
#[must_use]
pub fn frontier_refs(cert: &GenesisCert, state: &GovernanceState, set: &VerifiedSet) -> Vec<OpRef> {
    frontier(&set.ops, None)
        .into_iter()
        .filter_map(|op| author_record(cert, state, op.author()).map(|r| op.op_ref(&r)))
        .collect()
}

/// The state at `frontier`: the merge of the frontier ops' causal past
/// from the op store (the checkpoint replay; E3.5's electorate). A
/// frontier op not held is left out, as is everything only it reaches.
///
/// # Errors
/// No cert is pinned, or the op store failed.
pub async fn rebuild_at<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    frontier: &[OpRef],
) -> Result<GovernanceState, GovernanceRuntimeError> {
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let held = deps.governance_ops(community_id).await?;
    let set = verify_held(community_id, &held);
    Ok(merge::merge(&cert, &causal_past(&set.ops, frontier)))
}

/// The ops `frontier` reaches through deps (and `prev`), the frontier
/// included, in the set's order.
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

#[cfg(test)]
#[path = "op_log_tests.rs"]
mod tests;
