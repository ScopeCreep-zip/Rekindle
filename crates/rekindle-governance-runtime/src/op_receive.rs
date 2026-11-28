//! Receiving a governance op (plan E3.2, D15): the gossip fast path
//! ([`receive_op`]) and the op store's form of a fetched op
//! ([`held_from_encoded`]). An op this reader cannot interpret is held
//! too, so the rebuild can quarantine the community with a reason
//! ([`crate::quarantine::newly_uninterpretable`]) instead of dropping it.

use rekindle_codec::governance_op::{read_op, ReadOp};
use rekindle_governance::op::op_hash;
use rekindle_governance::state::GovernanceState;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::op_log::{verify_held_with, HeldOp, OpFate};

/// The op `encoded`, when it reads (an op this reader cannot interpret
/// included, which the rebuild holds quarantined and announces, D15) and
/// its hash is `op_ref`'s; `None` otherwise.
#[must_use]
pub fn held_from_encoded(op_ref: &OpRef, encoded: Vec<u8>) -> Option<HeldOp> {
    held_at(op_ref.op_hash, &op_ref.author_record, encoded)
}

/// The op `encoded`, found on `author_record`, when it reads and hashes to
/// `op_hash`: its signing device key, author and seq come from the op
/// itself (ADR 0022 rule 1), which a locator without an `OpRef` (an invite
/// link's op hash and seq) does not carry.
#[must_use]
pub fn held_at(op_hash: OpHash, author_record: &str, encoded: Vec<u8>) -> Option<HeldOp> {
    let envelope = OpEnvelope::read(&encoded)?;
    if envelope.hash != op_hash {
        return None;
    }
    Some(envelope.held(author_record.to_owned(), encoded))
}

/// What the op store needs of a received op, interpretable or not.
struct OpEnvelope {
    hash: OpHash,
    author: PseudonymKey,
    device_key: [u8; 32],
    seq: u64,
    issued_at: u64,
}

impl OpEnvelope {
    fn read(encoded: &[u8]) -> Option<Self> {
        Some(match read_op(encoded).ok()? {
            ReadOp::Known(d) => Self {
                hash: op_hash(&d.unsigned, &d.op.sig),
                author: d.op.author,
                device_key: d.op.device_key,
                seq: d.op.seq,
                issued_at: d.op.issued_at,
            },
            ReadOp::Unknown(u) => Self {
                hash: op_hash(&u.unsigned, &u.sig),
                author: u.author,
                device_key: u.device_key,
                seq: u.seq,
                issued_at: u.issued_at,
            },
        })
    }

    fn held(self, author_record: String, encoded: Vec<u8>) -> HeldOp {
        HeldOp {
            hash: self.hash,
            device_key: self.device_key,
            author: self.author,
            author_record,
            seq: self.seq,
            lamport: 0,
            issued_at: self.issued_at,
            encoded,
            fate: OpFate::PendingDeps,
        }
    }
}

/// The gossip fast path (`governanceOp @74`): store an op a member
/// published, as a fetched one, when it is new, its author's member record
/// is known (its own `MemberJoined`, or the merged state's) and it does not
/// verify as refused; then rebuild, which fetches its missing deps by
/// `OpRef`. An op this reader cannot interpret, by an author with a known
/// record, is stored too and the rebuild quarantines the community when a
/// member signed it (D15). `true`: it was stored.
///
/// # Errors
/// No cert is pinned, or the op store failed.
pub async fn receive_op<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    encoded: Vec<u8>,
) -> Result<bool, GovernanceRuntimeError> {
    let Ok(read) = read_op(&encoded) else {
        return Ok(false);
    };
    if matches!(read, ReadOp::Unknown(_)) {
        return receive_uninterpretable(deps, community_id, encoded).await;
    }
    if store_received_op(deps, community_id, encoded)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    crate::dht_hydration::rebuild_governance(deps, community_id).await;
    Ok(true)
}

/// Store an interpretable op received off our author logs (gossip, or a
/// frontier's fork proof) when it is new, its author's member record is
/// known and it does not verify as refused, with no rebuild: the caller
/// rebuilds. Its hash when stored.
///
/// # Errors
/// No cert is pinned, or the op store failed.
pub async fn store_received_op<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    encoded: Vec<u8>,
) -> Result<Option<OpHash>, GovernanceRuntimeError> {
    let Ok(ReadOp::Known(decoded)) = read_op(&encoded) else {
        return Ok(None);
    };
    let decoded = *decoded;
    let hash = op_hash(&decoded.unsigned, &decoded.op.sig);
    let mut held = deps.op_store().governance_ops(community_id).await?;
    if held.iter().any(|op| op.hash == hash) {
        return Ok(None);
    }
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let state = deps.governance_state(community_id).unwrap_or_default();
    let record = own_record(&decoded.op).or_else(|| {
        state.op_record(
            &cert,
            community_id,
            &decoded.op.author,
            &decoded.op.device_key,
        )
    });
    let Some(author_record) = record else {
        return Ok(None);
    };
    if !receive_bounded(&decoded.op, &state) {
        return Ok(None);
    }
    let mut op = HeldOp {
        hash,
        device_key: decoded.op.device_key,
        author: decoded.op.author.clone(),
        author_record,
        seq: decoded.op.seq,
        lamport: 0,
        issued_at: decoded.op.issued_at,
        encoded,
        fate: OpFate::PendingDeps,
    };
    held.push(op.clone());
    let set = verify_held_with(deps, community_id, &held);
    if set.quarantined.contains(&hash) {
        return Ok(None);
    }
    if let Some(verified) = set.ops.iter().find(|v| v.hash() == hash) {
        op.lamport = verified.lamport();
    }
    deps.op_store()
        .store_governance_op(community_id, &op)
        .await?;
    Ok(Some(hash))
}

/// The gossip path (`governanceOp @74`, lead ruling 2026-11-26, items 1
/// and 5): a gossiped op is only *seen*. It names where it lives (its
/// author's op log, ADR 0011's `OpRef` locator), and the gossip triggers a
/// read there at once; only what that read returns is received
/// ([`receive_op`]) and so merged, cited as a dep or named in a frontier.
/// A different op of the same device at that seq is a fork proof (D15).
/// An op the read does not return is not stored: a member that gossips an
/// op and never lands it leaves nothing a joiner's walk (plan D8, E3.2
/// S6 (4)) cannot fetch. When it lands later, the author's record watch
/// and the frontier sweep read it there. [`Gossiped::NotLanded`]: the
/// read did not return it yet (the gossip leaves before the author's
/// append lands, ruling item 2), so the caller reads its locator again
/// ([`LANDING_READS`]).
///
/// # Errors
/// No cert is pinned, the op store failed, or the read failed.
pub async fn receive_gossiped_op<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    gossiped: Vec<u8>,
) -> Result<Gossiped, GovernanceRuntimeError> {
    let Some(op_ref) = gossiped_locator(deps, community_id, &gossiped).await? else {
        return Ok(Gossiped::NotStored);
    };
    let logs = crate::op_walk::WalkLogCache::default();
    let at_locator = deps.op_store().fetch_governance_op(&op_ref, &logs).await?;
    let Some(at_locator) = at_locator else {
        tracing::debug!(
            community = community_id,
            op = %hex::encode(op_ref.op_hash.0),
            record = %op_ref.author_record,
            seq = op_ref.author_seq,
            "gossiped governance op seen, not on its author's log: not merged"
        );
        return Ok(Gossiped::NotLanded);
    };
    if held_from_encoded(&op_ref, at_locator.clone()).is_some() {
        return Ok(Gossiped::of(
            receive_op(deps, community_id, at_locator).await?,
        ));
    }
    // Another op of the same device at the same seq is on the log: the two
    // signed ops are a fork proof (D15), stored as a frontier's fork proof
    // is (`anti_entropy`), and the verify holds both as `Equivocation`,
    // never merged.
    let forked = OpEnvelope::read(&at_locator).is_some_and(|landed| {
        landed.author == op_ref.author
            && landed.seq == op_ref.author_seq
            && landed.device_key == op_ref.device_key
    });
    if !forked {
        return Ok(Gossiped::NotStored);
    }
    receive_op(deps, community_id, at_locator).await?;
    let proof = store_received_op(deps, community_id, gossiped).await?;
    if proof.is_some() {
        crate::dht_hydration::rebuild_governance(deps, community_id).await;
    }
    Ok(Gossiped::of(proof.is_some()))
}

/// What [`receive_gossiped_op`] did with a gossiped op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gossiped {
    /// The read at its locator returned it and it was stored.
    Stored,
    /// Not stored: held already, unreadable, its author's record unknown,
    /// or refused.
    NotStored,
    /// Its author's log does not serve it yet: seen, not merged.
    NotLanded,
}

impl Gossiped {
    fn of(stored: bool) -> Self {
        if stored {
            Self::Stored
        } else {
            Self::NotStored
        }
    }
}

/// The waits before each further read at a seen op's locator (ruling
/// 2026-11-26 item 1, "held as seen until a read at its locator returns
/// it"): the gossip leaves before the author's append, whose landing waits
/// on a DHT set (two set latencies, and the outbox's retries behind a
/// failed one), so the reads back off over about two minutes; past them, a
/// watched or swept frontier names the op once it lands.
pub const LANDING_READS: [std::time::Duration; 7] = [
    std::time::Duration::from_secs(1),
    std::time::Duration::from_secs(2),
    std::time::Duration::from_secs(4),
    std::time::Duration::from_secs(8),
    std::time::Duration::from_secs(16),
    std::time::Duration::from_secs(32),
    std::time::Duration::from_secs(64),
];

/// Where the gossiped op `encoded` lives, when it reads, is not held yet
/// and its author's record is known (its own `MemberJoined`'s record, or
/// the merged state's, as [`store_received_op`] resolves it).
async fn gossiped_locator<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    encoded: &[u8],
) -> Result<Option<OpRef>, GovernanceRuntimeError> {
    let Some(envelope) = OpEnvelope::read(encoded) else {
        return Ok(None);
    };
    let held = deps.op_store().governance_ops(community_id).await?;
    if held.iter().any(|op| op.hash == envelope.hash) {
        return Ok(None);
    }
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let state = deps.governance_state(community_id).unwrap_or_default();
    let named = match read_op(encoded) {
        Ok(ReadOp::Known(decoded)) => own_record(&decoded.op),
        _ => None,
    };
    let record = named
        .or_else(|| state.op_record(&cert, community_id, &envelope.author, &envelope.device_key));
    Ok(record.map(|author_record| OpRef {
        op_hash: envelope.hash,
        author: envelope.author,
        author_record,
        author_seq: envelope.seq,
        device_key: envelope.device_key,
    }))
}

/// The record an op names as its own, before any merge binds it: a
/// `MemberJoined` names its member record (op #0, or a re-join that binds
/// a fresh record, owner-proved), and a device's first op carries its own
/// `DeviceKey` endorsement (ADR 0022: "A device's first op (seq 0 in its
/// chain) is the op that carries its own `DeviceKey` endorsement"), whose
/// `deviceRecord` is the `M(p,d)` that device's chain lives on (ADR 0022
/// rule 3). A reader that resolved that op through the merged state alone
/// finds no binding yet and reads `M(p)`, where the op never is, so the
/// device's ops never land for it (ADR 0011: the `OpRef` locator is where
/// the op is). The endorsement itself is judged at the merge
/// (`DeviceView::absorb`); a false claim only names a record that does
/// not serve the op's hash.
fn own_record(op: &GovernanceOp) -> Option<String> {
    match &op.body {
        GovernanceEntry::MemberJoined { member_record, .. } => Some(member_record.clone()),
        GovernanceEntry::DeviceKey(e) if e.device_key == op.device_key => {
            Some(e.device_record.clone())
        }
        _ => None,
    }
}

/// [`receive_op`] for an op this reader cannot interpret: stored, when new
/// and its author's record is known in the merged state (it cannot be a
/// `MemberJoined` this reader reads), held for the rebuild to quarantine
/// and announce ([`crate::quarantine::newly_uninterpretable`]).
async fn receive_uninterpretable<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    encoded: Vec<u8>,
) -> Result<bool, GovernanceRuntimeError> {
    let Some(envelope) = OpEnvelope::read(&encoded) else {
        return Ok(false);
    };
    let held = deps.op_store().governance_ops(community_id).await?;
    if held.iter().any(|op| op.hash == envelope.hash) {
        return Ok(false);
    }
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let state = deps.governance_state(community_id).unwrap_or_default();
    let Some(record) = state.op_record(&cert, community_id, &envelope.author, &envelope.device_key)
    else {
        return Ok(false);
    };
    deps.op_store()
        .store_governance_op(community_id, &envelope.held(record, encoded))
        .await?;
    crate::dht_hydration::rebuild_governance(deps, community_id).await;
    Ok(true)
}

/// The receive-side bounds on a gossiped op, before it is stored: its
/// fields are within the reader bounds (`validate::bounds`, which the merge
/// applies too), and a `MemberJoined` (op #0 or a re-join) from an author
/// `state` does not already hold carries an invite proof naming one of the community's live
/// `InviteCreated` codes (plan E3.2: readers check `inviteProof` against the
/// community's `InviteCreated` set; every admission mode needs an invite),
/// so a key with no invite cannot leave a row behind. The merge's invite
/// gate still decides admission.
#[must_use]
pub fn receive_bounded(op: &GovernanceOp, state: &GovernanceState) -> bool {
    if rekindle_governance::validate::bounds::out_of_bounds_field(&op.body).is_some() {
        return false;
    }
    match &op.body {
        // Op #0, or a re-join by an author that left or was removed.
        GovernanceEntry::MemberJoined { invite_proof, .. } => {
            let known = state.members.contains_key(&op.author)
                || state.pending_members.contains_key(&op.author);
            known
                || invite_proof.as_ref().is_some_and(|proof| {
                    state
                        .invites
                        .values()
                        .any(|invite| invite.code_hash == proof.code_hash)
                })
        }
        _ => true,
    }
}
