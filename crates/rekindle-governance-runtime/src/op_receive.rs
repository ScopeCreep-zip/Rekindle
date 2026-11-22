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
    let envelope = OpEnvelope::read(&encoded)?;
    if envelope.hash != op_ref.op_hash {
        return None;
    }
    Some(envelope.held(op_ref.author_record.clone(), encoded))
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
    let mut held = deps.governance_ops(community_id).await?;
    if held.iter().any(|op| op.hash == hash) {
        return Ok(None);
    }
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let state = deps.governance_state(community_id).unwrap_or_default();
    let record = match &decoded.op.body {
        // A `MemberJoined` names its own record (op #0, or a re-join that
        // binds a fresh record, owner-proved).
        GovernanceEntry::MemberJoined { member_record, .. } => Some(member_record.clone()),
        _ => state.op_record(
            &cert,
            community_id,
            &decoded.op.author,
            &decoded.op.device_key,
        ),
    };
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
    deps.store_governance_op(community_id, &op).await?;
    Ok(Some(hash))
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
    let held = deps.governance_ops(community_id).await?;
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
    deps.store_governance_op(community_id, &envelope.held(record, encoded))
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
