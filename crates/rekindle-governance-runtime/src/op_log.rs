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

use std::collections::{BTreeSet, HashMap};

use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::merge::{self, MergeOutcome};
use rekindle_governance::op::{op_signing_bytes, VerifiedOp};
use rekindle_governance::power::{
    removal_target, seq_frontier, target_seq_frontier, with_target_seq_frontier,
};
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;
use rekindle_types::governance::{GenesisCert, GovernanceEntry, GovernanceOp, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
pub use crate::op_receive::{held_from_encoded, receive_bounded, receive_op};
pub use crate::op_walk::{walk, WalkLogCache};
pub use crate::own_frontier::{fit_fork_proof, frontier_refs, own_fork_proof, own_frontier};
pub use crate::verify_cache::verify_held_cached;
pub use rekindle_governance::checkpoint::causal_past;

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
    /// The device key that signed the op (ADR 0022 rule 1).
    pub device_key: [u8; 32],
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

/// Decode and verify `held` for community `anchor` in dependency order,
/// with no kept verdicts: a read outside a community's lifetime (a test, a
/// projection). A community's reads go through its
/// [`OpVerifyCache`](crate::verify_cache::OpVerifyCache) instead
/// ([`verify_held_cached`]).
#[must_use]
pub fn verify_held(anchor: &str, held: &[HeldOp]) -> VerifiedSet {
    verify_held_cached(anchor, held, &crate::verify_cache::OpVerifyCache::default())
}

/// [`verify_held`] through `community_id`'s kept verdicts
/// (`GovernanceRuntimeDeps::op_verify_cache`).
#[must_use]
pub fn verify_held_with<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    held: &[HeldOp],
) -> VerifiedSet {
    verify_held_cached(community_id, held, &deps.op_verify_cache(community_id))
}

/// The member record an author's ops live in: the cert's for the
/// creator, else the author's `MemberJoined` in the merged state, kept
/// after the author leaves or is removed (`op_log_records`).
#[must_use]
pub fn author_record(
    cert: &GenesisCert,
    state: &GovernanceState,
    author: &PseudonymKey,
) -> Option<String> {
    state.author_record(cert, author)
}

/// Who we are in a community and our member record in `state`.
pub struct OwnMember {
    /// Our pseudonym in the community.
    pub pseudonym: PseudonymKey,
    /// The same, hex, as the membership holds it.
    pub hex: String,
    /// Our member record in `state` ([`author_record`]): `None` when the
    /// merge never admitted our `MemberJoined`.
    pub record: Option<String>,
}

/// Our pseudonym here (the membership's, every op of ours is signed as it)
/// and its hex; needs no pinned cert, unlike [`own_member`].
///
/// # Errors
/// No membership or pseudonym for the community, or a malformed one.
pub fn own_pseudonym<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<(PseudonymKey, String), GovernanceRuntimeError> {
    let hex = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?
        .my_pseudonym_hex
        .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.to_string()))?;
    let bytes: [u8; 32] = hex::decode(&hex)
        .map_err(|e| GovernanceRuntimeError::InvalidPseudonymHex(e.to_string()))?
        .try_into()
        .map_err(|_| {
            GovernanceRuntimeError::InvalidPseudonymHex("pseudonym must be 32 bytes".into())
        })?;
    Ok((PseudonymKey(bytes), hex))
}

/// Our pseudonym (the membership's, the one every op of ours is signed
/// as) and our member record in `state` under the pinned cert: the one
/// answer to "who are we here, and are we admitted" (plan SU1). A caller
/// that acts on the answer reads `state` under the rebuild lock, so no
/// rebuild installs between the answer and the act.
///
/// # Errors
/// No membership, pseudonym or pinned cert for the community, or a
/// malformed pseudonym.
pub fn own_member<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    state: &GovernanceState,
) -> Result<OwnMember, GovernanceRuntimeError> {
    let (pseudonym, hex) = own_pseudonym(deps, community_id)?;
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_string()))?;
    let record = author_record(&cert, state, &pseudonym);
    Ok(OwnMember {
        pseudonym,
        hex,
        record,
    })
}

/// The frontier: the verified ops no other verified op cites, newest
/// first by `(lamport, hash)`, at most [`MAX_FRONTIER_REFS`], always
/// holding `must` (our previous op, which the chain rule requires among
/// the deps). Returned in hash order, the canonical order on the wire.
pub(crate) fn frontier(ops: &[VerifiedOp], must: Option<OpHash>) -> Vec<&VerifiedOp> {
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

/// The deps an op cites (plan E3.2 S7c.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deps {
    /// The frontier of the verified set (every op but a join).
    Frontier,
    /// A `MemberJoined`'s shallow deps, by hash, from
    /// [`join_deps`](crate::join_deps::join_deps); our previous op is
    /// added when missing (the chain rule).
    Join(Vec<OpHash>),
    /// A `Checkpoint`'s deps (plan E3.2 S7c.4): its cut's heads, by hash;
    /// our previous op is added when missing.
    Cited(Vec<OpHash>),
    /// The frontier and the ops named, by hash: a transfer acceptance
    /// cites the offer it accepts (plan E3.2 S7s.4, `successor_gate`).
    FrontierAnd(Vec<OpHash>),
}

/// Our op for `body`: the next seq of our chain, `deps` as its deps,
/// signed by `key` (our community pseudonym) and verified against `set`
/// as every reader verifies it. A `MemberRemoved` or `BanEntry` gets its
/// target's seq frontier from those deps, whatever `body` held.
///
/// # Errors
/// A dep's author has no member record in `state`, a join dep is not in
/// `set`, the op does not encode, or it fails verification.
pub fn build_op(
    anchor: &str,
    cert: &GenesisCert,
    state: &GovernanceState,
    set: &VerifiedSet,
    key: &SigningKey,
    body: GovernanceEntry,
    deps: Deps,
    issued_at: u64,
) -> Result<(VerifiedOp, Vec<u8>), GovernanceRuntimeError> {
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let last = set
        .ops
        .iter()
        .filter(|op| *op.author() == author)
        .max_by_key(|op| op.op().seq);
    let (seq, prev) = last.map_or((0, None), |op| (op.op().seq + 1, Some(op.hash())));
    let cited = cited(set, deps, prev)?;
    let known: HashMap<OpHash, &VerifiedOp> = set.ops.iter().map(|o| (o.hash(), o)).collect();
    // A removal carries its target's seq frontier in these deps (plan E3.2
    // S7c.3a): every reader refuses one whose field differs from it.
    let body = match target_seq_frontier(&body).and(removal_target(&body)) {
        Some(target) => {
            let frontier = seq_frontier(
                cited.iter().map(|op| op.hash()),
                |h| known.get(h).copied(),
                target,
            );
            with_target_seq_frontier(body, frontier)
        }
        None => body,
    };
    let deps = cited
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
        device_key: author.0,
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
    let verified = VerifiedOp::verify(anchor, op, &unsigned, |h| known.get(h).copied())
        .map_err(|e| GovernanceRuntimeError::OpRefused(format!("{e:?}")))?;
    Ok((verified, encoded))
}

/// The ops `deps` names in `set`, `prev` among them, in hash order.
fn cited(
    set: &VerifiedSet,
    deps: Deps,
    prev: Option<OpHash>,
) -> Result<Vec<&VerifiedOp>, GovernanceRuntimeError> {
    let mut hashes = match deps {
        Deps::Frontier => return Ok(frontier(&set.ops, prev)),
        Deps::FrontierAnd(extra) => (frontier(&set.ops, prev).iter().map(|op| op.hash()))
            .chain(extra)
            .collect(),
        Deps::Join(hashes) | Deps::Cited(hashes) => hashes,
    };
    hashes.extend(prev);
    hashes.sort_unstable();
    hashes.dedup();
    hashes
        .iter()
        .map(|h| {
            set.ops.iter().find(|op| op.hash() == *h).ok_or_else(|| {
                GovernanceRuntimeError::OpRefused(format!("dep {} not held", hex::encode(h.0)))
            })
        })
        .collect()
}

/// The stored form of a verified op.
#[must_use]
pub fn held(op: &VerifiedOp, author_record: String, encoded: Vec<u8>, fate: OpFate) -> HeldOp {
    HeldOp {
        hash: op.hash(),
        device_key: op.op().device_key,
        author: op.author().clone(),
        author_record,
        seq: op.op().seq,
        lamport: op.lamport(),
        issued_at: op.op().issued_at,
        encoded,
        fate,
    }
}

/// One held op's fate after a merge, as `governance_ops` records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpVerdict {
    pub hash: OpHash,
    pub fate: OpFate,
    /// The validated lamport (`1 + max(dep lamports)`) of an op that
    /// verified; `None` for one that did not, whose lamport is unknown.
    pub lamport: Option<u64>,
}

/// The fate of every held op after `outcome`: an equivocator's ops from
/// its fork seq on (the ones the merge dropped; its earlier ops keep their
/// merge fate, D15) are `Equivocation`, an op the merge refused (validation, strong removal, the
/// invite and approval gates) or masked after its author's removal is
/// `Quarantined`, one the merge holds for a
/// dep is `PendingDeps`, and only an applied op is `Applied`.
#[must_use]
pub fn fates(set: &VerifiedSet, outcome: &MergeOutcome) -> Vec<OpVerdict> {
    // A masked op (after its author's removal, plan E3.2 S7c.3a) is held
    // quarantined like a refused one.
    let refused: BTreeSet<OpHash> = (outcome.refused.iter().copied())
        .chain(outcome.quarantined.iter().map(|(hash, _)| *hash))
        .collect();
    let pending: BTreeSet<OpHash> = outcome.pending.iter().copied().collect();
    let mut out: Vec<OpVerdict> = set
        .ops
        .iter()
        .map(|op| {
            let hash = op.hash();
            let forked = (outcome.equivocators.iter())
                .any(|(author, fork)| author == op.author() && op.op().seq >= *fork);
            let fate = if forked {
                OpFate::Equivocation
            } else if refused.contains(&hash) {
                OpFate::Quarantined
            } else if pending.contains(&hash) {
                OpFate::PendingDeps
            } else {
                OpFate::Applied
            };
            OpVerdict {
                hash,
                fate,
                lamport: Some(op.lamport()),
            }
        })
        .collect();
    let unverified = |fate| {
        move |hash: &OpHash| OpVerdict {
            hash: *hash,
            fate,
            lamport: None,
        }
    };
    out.extend(set.pending.iter().map(unverified(OpFate::PendingDeps)));
    out.extend(set.quarantined.iter().map(unverified(OpFate::Quarantined)));
    out
}

/// Re-read `community_id`'s op set from the op store, fetch every missing
/// dep by its `OpRef`, at the cited record and then at its author's other
/// records ([`locators`](crate::op_walk::locators)), until a pass fetches
/// nothing new, and merge under
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
    let mut tried: BTreeSet<(OpHash, String)> = BTreeSet::new();
    let logs = WalkLogCache::default();
    loop {
        let set = verify_held_with(deps, community_id, &held);
        // Each missing dep at its cited record and at every other record
        // of its author we hold an op from, as the join walk looks
        // (`op_walk::locators`): a re-joiner's earlier ops are cited at
        // its first record, which may never have held them.
        let wanted: Vec<OpRef> = crate::op_walk::locators(&set.missing, &held)
            .into_iter()
            .filter(|r| tried.insert((r.op_hash, r.author_record.clone())))
            .collect();
        let mut fetched = false;
        for op_ref in wanted {
            // Two locators of one op may both answer.
            if held.iter().any(|op| op.hash == op_ref.op_hash) {
                continue;
            }
            let Some(op) = fetch_held(deps, &op_ref, &logs).await? else {
                continue;
            };
            deps.store_governance_op(community_id, &op).await?;
            held.push(op);
            fetched = true;
        }
        if !fetched {
            let outcome = merge::merge_outcome(&cert, &set.ops);
            for (author, fork) in &outcome.equivocators {
                tracing::warn!(
                    community = community_id,
                    author = %hex::encode(author.0),
                    fork,
                    "governance equivocation: two ops at one seq; the author's ops from the fork on are dropped"
                );
            }
            let verdicts = fates(&set, &outcome);
            warn_newly_dropped(community_id, &held, &verdicts);
            deps.set_governance_op_fates(community_id, verdicts).await?;
            crate::checkpoint_rows::record(deps, community_id, &set.ops, &outcome).await?;
            crate::partial::finish_backfill(deps, community_id, &cert, &set).await?;
            let quarantined = crate::quarantine::newly_quarantined(community_id, &held, &outcome);
            let unknown =
                crate::quarantine::newly_uninterpretable(community_id, &held, &outcome.state);
            for event in quarantined.into_iter().chain(unknown) {
                deps.emit_event(event);
            }
            crate::successor::announce(deps, community_id, &outcome.state).await;
            return Ok(outcome.state);
        }
    }
}

/// One WARN per op this merge newly drops for its author's equivocation
/// (D15: the fork and every later op of that author), naming the op by
/// its hash, author and seq; an op already held as dropped stays quiet.
fn warn_newly_dropped(community_id: &str, held: &[HeldOp], verdicts: &[OpVerdict]) {
    for verdict in verdicts.iter().filter(|v| v.fate == OpFate::Equivocation) {
        let Some(op) = held.iter().find(|h| h.hash == verdict.hash) else {
            continue;
        };
        if op.fate != OpFate::Equivocation {
            tracing::warn!(
                community = community_id,
                op = %hex::encode(op.hash.0),
                author = %hex::encode(op.author.0),
                seq = op.seq,
                "governance equivocation: an op of an equivocating author is dropped"
            );
        }
    }
}

/// The op `op_ref` names, fetched from its author's op log and checked
/// against the locator's hash; `None` when it is not there or does not
/// match. `logs` keeps the author logs this walk, rebuild or sweep has
/// read, so each is read once.
///
/// # Errors
/// The fetch failed.
pub async fn fetch_held<D: GovernanceRuntimeDeps>(
    deps: &D,
    op_ref: &OpRef,
    logs: &WalkLogCache,
) -> Result<Option<HeldOp>, GovernanceRuntimeError> {
    let Some(encoded) = deps.fetch_governance_op(op_ref, logs).await? else {
        return Ok(None);
    };
    Ok(held_from_encoded(op_ref, encoded))
}

/// The `OpRef` of held op `hash` of `community_id`, as a dep cites it.
///
/// # Errors
/// No cert is pinned, the op store failed, or the op is not held (or its
/// author has no member record).
pub async fn held_op_ref<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    hash: OpHash,
) -> Result<OpRef, GovernanceRuntimeError> {
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_owned()))?;
    let held = deps.governance_ops(community_id).await?;
    let set = verify_held_with(deps, community_id, &held);
    set.ops
        .iter()
        .find(|op| op.hash() == hash)
        .and_then(|op| author_record(&cert, &state, op.author()).map(|r| op.op_ref(&r)))
        .ok_or_else(|| {
            GovernanceRuntimeError::OpRefused(format!("op {} not held", hex::encode(hash.0)))
        })
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
    let set = verify_held_with(deps, community_id, &held);
    Ok(merge::merge(&cert, &causal_past(&set.ops, frontier)))
}

#[cfg(test)]
#[path = "op_log_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "record_binding_tests.rs"]
mod record_binding_tests;
