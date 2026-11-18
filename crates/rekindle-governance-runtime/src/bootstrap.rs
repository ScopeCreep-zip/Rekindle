//! The bootstrap exchange of the self-sovereign join (plan E3.2, ADR 0011).
//!
//! The join works from the DHT with nobody online; when the inviter is
//! online, this exchange is how it learns of the joiner. The joiner sends
//! `BootstrapRequest{joinerPseudonym, anchor, memberJoined}` as a bare
//! `app_call` to `InviteSecrets.inviter_route_blob` once its `MemberJoined`
//! is durable on its member record. The request's authority is that
//! signed op, carried with its `OpRef` (an inviter answers within the
//! app_call reply window, too short for a DHT read): the inviter checks
//! it against the `OpRef`, verifies it,
//! stores and merges it, and only a joiner who is then a member of the
//! merged state is answered, with `bootstrapResponseV2{anchor, frontier}`:
//! where the joiner's walk of the op DAG continues. It ships no keys (D11;
//! the joiner asks for the community key by `RequestMEK`), no messages and
//! no checkpoint hint (the joiner asks for that before its walk, by
//! `checkpointHintRequest`).

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::governance::OpRef;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::op_log::{self, HeldOp};

/// The `app_call` reply cap a response stays under (Veilid's
/// `MAX_APP_CALL_Q_MESSAGE_LEN`).
pub const MAX_BOOTSTRAP_RESPONSE_LEN: usize = 32_768;

/// Who asks: the joiner's pseudonym and its `MemberJoined` (op #0, or a
/// re-join's later op).
#[derive(Debug, Clone)]
pub struct BootstrapJoiner {
    /// Hex of the joiner's community pseudonym.
    pub pseudonym: String,
    pub member_joined: OpRef,
    /// The encoded signed op `member_joined` names.
    pub member_joined_op: Vec<u8>,
    /// The joiner's route, where the inviter delivers the community key
    /// (D16).
    pub route: Vec<u8>,
}

fn adapter(msg: impl Into<String>) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(msg.into())
}

/// Answer a joiner's `BootstrapRequest`: check its `MemberJoined` against
/// the `OpRef`, verify, store and merge it, then encode
/// `bootstrapResponseV2` with our frontier.
///
/// # Errors
/// We are not in the community, the op is not the joiner's `MemberJoined`
/// or does not match its `OpRef`, the merged state does not hold the joiner as a member,
/// or the store or the encode failed. The caller NAKs.
pub async fn build_bootstrap_response_v2<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    joiner: &BootstrapJoiner,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    deps.community_membership(anchor)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(anchor.to_owned()))?;
    let author = &joiner.member_joined.author;
    if hex::encode(author.0) != joiner.pseudonym {
        return Err(adapter("bootstrap: the OpRef is not the joiner's"));
    }
    let op = op_log::held_from_encoded(&joiner.member_joined, joiner.member_joined_op.clone())
        .ok_or_else(|| adapter("bootstrap: the op does not match the joiner's OpRef"))?;
    let before = deps.governance_state(anchor).unwrap_or_default();
    if !is_joiners_member_joined(&op, &joiner.member_joined, &before) {
        return Err(adapter(
            "bootstrap: the op is not the joiner's MemberJoined (op #0, or a re-join after its departure)",
        ));
    }
    op_log::receive_op(deps, anchor, op.encoded).await?;
    let state = deps
        .governance_state(anchor)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(anchor.to_owned()))?;
    if !state.members.contains_key(author) || state.bans.contains(author) {
        return Err(adapter(
            "bootstrap: the joiner is not a member of the merged state",
        ));
    }
    let cert = deps
        .genesis_cert(anchor)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(anchor.to_owned()))?;
    let set = op_log::verify_held_with(deps, anchor, &deps.governance_ops(anchor).await?);
    let payload = ControlPayload::BootstrapResponseV2 {
        anchor: anchor.to_owned(),
        frontier: op_log::frontier_refs(&cert, &state, &set),
    };
    let bytes = encode_response(payload)?;
    deps.emit_event(GovernanceRuntimeEvent::BootstrapResponseBuilt {
        community_id: anchor.to_owned(),
        joiner_pseudonym_hex: joiner.pseudonym.clone(),
        bytes: bytes.len(),
    });
    Ok(bytes)
}

/// Whether `op` is the joiner's `MemberJoined`: its op #0, or a re-join,
/// a later `MemberJoined` of the same pseudonym by an author `state` does
/// not hold as a member (it left or was removed; the merge's re-join rule,
/// `gates::rejoined`, decides whether it stands). A member's community
/// pseudonym is fixed per identity and community (`derive_community_pseudonym`),
/// so a re-join continues its own op chain.
fn is_joiners_member_joined(
    op: &op_log::HeldOp,
    op_ref: &OpRef,
    state: &rekindle_governance::state::GovernanceState,
) -> bool {
    let Ok(decoded) = rekindle_codec::governance_op::decode_op(&op.encoded) else {
        return false;
    };
    let joined = matches!(
        decoded.op.body,
        rekindle_types::governance::GovernanceEntry::MemberJoined { .. }
    );
    joined
        && op.author == op_ref.author
        && op.seq == op_ref.author_seq
        && (op.seq == 0 || !state.members.contains_key(&op.author))
}

fn encode_response(payload: ControlPayload) -> Result<Vec<u8>, GovernanceRuntimeError> {
    let bytes = rekindle_codec::capnp_envelope::encode_community_envelope(
        &CommunityEnvelope::Control(payload),
    )
    .map_err(|e| GovernanceRuntimeError::Encoding(format!("encode bootstrap response: {e}")))?;
    if bytes.len() > MAX_BOOTSTRAP_RESPONSE_LEN {
        return Err(GovernanceRuntimeError::Encoding(format!(
            "bootstrap response is {} B, over the {MAX_BOOTSTRAP_RESPONSE_LEN} B app_call cap",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Ask the inviter at `inviter_route` for its frontier once our
/// `MemberJoined` is durable, and walk the op DAG from it: the ops the
/// walk found (the caller stores the new ones and rebuilds).
///
/// # Errors
/// The `app_call` failed, the inviter refused
/// ([`GovernanceRuntimeError::BootstrapRefused`]: the reply is not a
/// `bootstrapResponseV2` for `anchor`), or the walk failed.
pub async fn request_bootstrap<D: GovernanceRuntimeDeps>(
    deps: &D,
    inviter_route: &[u8],
    anchor: &str,
    joiner: &BootstrapJoiner,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
    let request = CommunityEnvelope::Control(ControlPayload::BootstrapRequest {
        joiner_pseudonym: joiner.pseudonym.clone(),
        anchor: anchor.to_owned(),
        member_joined: joiner.member_joined.clone(),
        member_joined_op: joiner.member_joined_op.clone(),
        joiner_route: joiner.route.clone(),
    });
    let bytes = rekindle_codec::capnp_envelope::encode_community_envelope(&request)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("encode bootstrap request: {e}")))?;
    let reply = deps.app_call_peer(inviter_route, bytes).await?;
    let frontier = match rekindle_codec::capnp_envelope::try_decode_community_envelope(&reply) {
        Ok(Some(CommunityEnvelope::Control(ControlPayload::BootstrapResponseV2 {
            anchor: answered,
            frontier,
            ..
        }))) if answered == anchor => frontier,
        _ => {
            return Err(GovernanceRuntimeError::BootstrapRefused(
                "the reply is not a bootstrapResponseV2 for this community".into(),
            ))
        }
    };
    op_log::walk(deps, anchor, &frontier).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;
    use rekindle_types::governance::OpHash;
    use rekindle_types::id::PseudonymKey;

    /// A Veilid record key at its longest text form.
    fn record_key(i: u8) -> String {
        format!(
            "VLD0:{}:{}",
            "A".repeat(43),
            char::from(b'a' + i % 26).to_string().repeat(43)
        )
    }

    fn op_ref(i: u8) -> OpRef {
        OpRef {
            op_hash: OpHash([i; 32]),
            author: PseudonymKey([i.wrapping_add(1); 32]),
            author_record: record_key(i),
            author_seq: u64::MAX,
        }
    }

    #[test]
    fn the_largest_response_fits_the_app_call_cap() {
        let max_frontier = u8::try_from(MAX_FRONTIER_REFS).unwrap();
        let payload = ControlPayload::BootstrapResponseV2 {
            anchor: record_key(200),
            frontier: (0..max_frontier).map(op_ref).collect(),
        };
        let bytes = encode_response(payload.clone()).expect("fits");
        assert!(
            bytes.len() < MAX_BOOTSTRAP_RESPONSE_LEN,
            "{} B",
            bytes.len()
        );
        let back = rekindle_codec::capnp_envelope::try_decode_community_envelope(&bytes)
            .unwrap()
            .unwrap();
        let CommunityEnvelope::Control(ControlPayload::BootstrapResponseV2 { frontier, .. }) = back
        else {
            panic!("not a bootstrapResponseV2");
        };
        assert_eq!(frontier.len(), MAX_FRONTIER_REFS);
    }

    #[test]
    fn a_frontier_over_the_bound_is_refused() {
        let over = u8::try_from(MAX_FRONTIER_REFS + 1).unwrap();
        let payload = ControlPayload::BootstrapResponseV2 {
            anchor: record_key(0),
            frontier: (0..over).map(op_ref).collect(),
        };
        assert!(encode_response(payload).is_err());
    }
}
