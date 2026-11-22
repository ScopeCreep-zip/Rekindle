//! A checkpoint hint, asked for and answered (plan E3.2 S7c.5, "Using a
//! checkpoint"): before its walk a joiner asks its inviter, authorised by
//! the invite it holds (the Host admits the requester and rate-limits it
//! first), for the usable hint selected at the greatest height and, height
//! by height, the hint each `prevCheckpoint` names, with the endorsements a
//! strict majority of each electorate needs ([`answer_hint_request`],
//! [`request_hint`]). A community holding a checkpoint equivocation answers
//! no hint (S7c.4: readers use none).

use rekindle_codec::capnp_envelope::{encode_community_envelope, try_decode_community_envelope};
use rekindle_codec::checkpoint_hint::{
    decode_checkpoint_hint_response, CheckpointHintRequest, CheckpointHintResponse,
    MAX_HINT_ENDORSEMENTS,
};
use rekindle_codec::checkpoint_state::PreJoinRequest;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::checkpoint::{checkpoint_digest, is_strict_majority, Hints, UsableHint};
use rekindle_governance::merge::merge_outcome;
use rekindle_governance::op::VerifiedOp;
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GenesisCert, GovernanceEntry, OpRef};

use crate::deps::GovernanceRuntimeDeps;
use crate::op_log::verify_held_with;
use crate::GovernanceRuntimeError;

fn encoding(what: &str, e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Encoding(format!("{what}: {e}"))
}

/// The hint `height` (0: the one selected at the greatest height) and
/// `digest` name among `hints`; none under an equivocation.
fn chosen<'a>(hints: &'a Hints, height: u64, digest: &[u8; 32]) -> Option<&'a UsableHint> {
    if !hints.equivocations.is_empty() {
        return None;
    }
    if height == 0 {
        return hints.latest();
    }
    (hints.usable.iter()).find(|h| h.height == height && h.digest == *digest)
}

/// The answer to a hint request over the merged op set `ops`: the hint's
/// lowest op and, of its endorsers' ops in its round, the lowest
/// `(lamport, op_hash)` one per endorser, lowest first, until they are a
/// strict majority of its electorate. No hint when none is chosen, when
/// the majority needs more than [`MAX_HINT_ENDORSEMENTS`] refs, or when an
/// op's author has no record to locate it by.
#[must_use]
pub fn hint_response(
    cert: &GenesisCert,
    state: &GovernanceState,
    ops: &[VerifiedOp],
    hints: &Hints,
    (height, digest): (u64, &[u8; 32]),
) -> CheckpointHintResponse {
    let Some(hint) = chosen(hints, height, digest) else {
        return CheckpointHintResponse::default();
    };
    let mut endorsing: Vec<&VerifiedOp> = (ops.iter())
        .filter(|op| hint.endorsers.contains(&op.author().0))
        .filter(|op| match op.body() {
            GovernanceEntry::Checkpoint(c) => {
                c.height == hint.height
                    && c.round == hint.round
                    && checkpoint_digest(c) == hint.digest
            }
            _ => false,
        })
        .collect();
    endorsing.sort_by_key(|op| (op.lamport(), op.hash()));
    let mut seen = std::collections::BTreeSet::new();
    endorsing.retain(|op| seen.insert(op.author().0));
    let need = hint.electorate / 2 + 1;
    endorsing.truncate(need);
    let locate = |op: &VerifiedOp| -> Option<OpRef> {
        Some(op.op_ref(&crate::op_log::op_record(cert, state, op)?))
    };
    let first = ops.iter().find(|op| op.hash() == hint.op);
    let (Some(checkpoint), Some(endorsements)) = (
        first.and_then(locate),
        endorsing
            .into_iter()
            .map(locate)
            .collect::<Option<Vec<_>>>(),
    ) else {
        return CheckpointHintResponse::default();
    };
    if endorsements.len() > MAX_HINT_ENDORSEMENTS
        || !is_strict_majority(endorsements.len(), hint.electorate)
    {
        return CheckpointHintResponse::default();
    }
    CheckpointHintResponse {
        checkpoint: Some(checkpoint),
        endorsements,
    }
}

/// Answer an admitted requester's verified hint `request`: the encoded
/// `checkpointHintResponse` envelope.
///
/// # Errors
/// The community is partial (S7c.8 (1)), no cert is pinned, the op store
/// failed, or encoding failed.
pub async fn answer_hint_request<D: GovernanceRuntimeDeps>(
    deps: &D,
    request: &CheckpointHintRequest,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    let community = request.community.as_str();
    crate::partial::refuse_while_partial(deps, community).await?;
    let cert = deps
        .genesis_cert(community)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community.to_owned()))?;
    let held = deps.op_store().governance_ops(community).await?;
    let set = verify_held_with(deps, community, &held);
    let outcome = merge_outcome(&cert, &set.ops);
    let response = hint_response(
        &cert,
        &outcome.state,
        &set.ops,
        &outcome.hints,
        (request.height, &request.digest),
    );
    let response = response
        .encode()
        .map_err(|e| encoding("checkpoint hint response", e))?;
    encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::CheckpointHintResponse { response },
    ))
    .map_err(|e| encoding("checkpoint hint envelope", e))
}

/// Ask the peer at `route` for the hint `height` (0: the selected one at
/// the greatest height) and `digest` name, as `me` (our pseudonym in
/// `community`), a member or, with `invite_code`, a joiner holding that
/// invite (its request tagged under the code, plan E3.2 S7c.8 (2)).
///
/// # Errors
/// Encoding failed, the call failed, or the reply is not a hint response.
pub async fn request_hint<D: GovernanceRuntimeDeps>(
    deps: &D,
    route: &[u8],
    community: &str,
    me: &SigningKey,
    (height, digest): (u64, [u8; 32]),
    invite_code: Option<&str>,
) -> Result<CheckpointHintResponse, GovernanceRuntimeError> {
    let request = CheckpointHintRequest {
        community: community.to_owned(),
        height,
        digest,
        requester: me.verifying_key().to_bytes(),
        invite: None,
    };
    let request = match invite_code {
        Some(code) => request.with_invite_code(code),
        None => request,
    }
    .encode(me)
    .map_err(|e| encoding("checkpoint hint request", e))?;
    let envelope = encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::CheckpointHintRequest { request },
    ))
    .map_err(|e| encoding("checkpoint hint envelope", e))?;
    let reply = deps.app_call_peer(route, envelope).await?;
    match try_decode_community_envelope(&reply) {
        Ok(Some(CommunityEnvelope::Control(ControlPayload::CheckpointHintResponse {
            response,
        }))) => decode_checkpoint_hint_response(&response)
            .map_err(|e| encoding("checkpoint hint response", e)),
        _ => Err(GovernanceRuntimeError::Adapter(
            "the reply is not a checkpointHintResponse".into(),
        )),
    }
}

#[cfg(test)]
#[path = "checkpoint_hint_tests.rs"]
mod tests;
