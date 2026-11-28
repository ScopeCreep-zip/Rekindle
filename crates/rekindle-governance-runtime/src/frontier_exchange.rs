//! The frontier request and its answer (plan E3.2 S7d-keys.8, the unlock
//! barrier). A ring peer answers an admitted member (the Host admits the
//! requester and rate-limits it first, like the `headAdvanced` handler)
//! with its own `Frontier`: the heads of its verified op set (at most
//! `MAX_FRONTIER_REFS`), D15's roster checksum and its fork refs, never
//! the fork proof ops, so the answer stays under the 32,768 B `app_call`
//! cap ([`answer_frontier_request`]). The asker ([`request_frontier`])
//! fetches every head from its author's own member record and checks it
//! there, so a peer cannot raise the barrier's result with a head it made
//! up: frontiers are tips, and the barrier walks from them
//! (`op_log::walk`) for the highest seq of its device key.

use std::future::Future;

use rekindle_codec::capnp_envelope::{encode_community_envelope, try_decode_community_envelope};
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::frontier_request::{
    decode_frontier_response, encode_frontier_response, FrontierRequest,
};
use rekindle_codec::governance_op::decode_op;
use rekindle_governance::op::{op_hash, op_signature_holds};
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{Frontier, GenesisCert, OpRef};

use crate::deps::GovernanceRuntimeDeps;
use crate::op_log::{held_from_encoded, own_frontier, verify_held_with, HeldOp};
use crate::op_walk::WalkLogCache;
use crate::GovernanceRuntimeError;

fn encoding(what: &str, e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Encoding(format!("{what}: {e}"))
}

/// A ring peer's answer, every head fetched from its author's member
/// record and checked: the frontier as it came, and the head ops in its
/// order, the tips the barrier walks from.
#[derive(Debug)]
pub struct PeerFrontier {
    pub frontier: Frontier,
    pub heads: Vec<HeldOp>,
}

/// Answer an admitted requester's verified `request`: the encoded
/// `frontierResponse` envelope carrying our own `Frontier` without fork
/// proof ops (the fork stays named by its refs).
///
/// # Errors
/// No cert is pinned, the op store failed, or encoding failed (over the
/// response cap).
pub async fn answer_frontier_request<D: GovernanceRuntimeDeps>(
    deps: &D,
    request: &FrontierRequest,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    let community = request.community.as_str();
    let cert = deps
        .genesis_cert(community)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community.to_owned()))?;
    let held = deps.op_store().governance_ops(community).await?;
    let set = verify_held_with(deps, community, &held);
    let mut frontier = own_frontier(&cert, &set, &held, None);
    frontier.fork_proof.clear();
    let frontier =
        encode_frontier_response(&frontier).map_err(|e| encoding("frontier response", e))?;
    encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::FrontierResponse { frontier },
    ))
    .map_err(|e| encoding("frontier envelope", e))
}

/// Whether `encoded`, read at `head`'s locator, is the op `head` names in
/// `community`: it decodes, its device key signed it, it hashes to the
/// head's hash, and its author, seq and device key are the head's; and,
/// when `known` (our cert and state) names the record the op lives on
/// (its device's `M(p,d)` or its author's member record, ADR 0022 rule 3),
/// the head points into that record.
#[must_use]
pub fn head_holds(
    community: &str,
    head: &OpRef,
    encoded: &[u8],
    known: Option<(&GenesisCert, &GovernanceState)>,
) -> bool {
    let Ok(decoded) = decode_op(encoded) else {
        return false;
    };
    let op = &decoded.op;
    let record_matches = known
        .and_then(|(cert, state)| state.op_record(cert, community, &op.author, &op.device_key))
        .is_none_or(|record| record == head.author_record);
    record_matches
        && op.author == head.author
        && op.seq == head.author_seq
        && op.device_key == head.device_key
        && op_hash(&decoded.unsigned, &op.sig) == head.op_hash
        && op_signature_holds(community, &op.device_key, &decoded.unsigned, &op.sig)
}

/// Check every head of `frontier`, a ring peer's answer for `community`,
/// against its author's member record: fetched at its locator and held by
/// [`head_holds`].
///
/// # Errors
/// A head is not on its author's record or does not hold there (a forged
/// head refuses the whole answer), or a fetch failed.
pub async fn verify_frontier_heads<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    frontier: Frontier,
) -> Result<PeerFrontier, GovernanceRuntimeError> {
    let cert = deps.genesis_cert(community);
    let state = deps.governance_state(community);
    let known = cert.as_ref().zip(state.as_ref());
    let logs = WalkLogCache::default();
    let mut heads = Vec::with_capacity(frontier.heads.len());
    for head in &frontier.heads {
        let encoded = deps.op_store().fetch_governance_op(head, &logs).await?;
        let held = encoded
            .filter(|encoded| head_holds(community, head, encoded, known))
            .and_then(|encoded| held_from_encoded(head, encoded));
        let Some(held) = held else {
            return Err(GovernanceRuntimeError::OpRefused(format!(
                "frontier head {} does not hold on its author's record",
                hex::encode(head.op_hash.0)
            )));
        };
        heads.push(held);
    }
    Ok(PeerFrontier { frontier, heads })
}

/// Ask the ring peer through `call` for its frontier of `community`, as `me`
/// (our pseudonym there), and check every head it names
/// ([`verify_frontier_heads`]). The unlock barrier (plan E3.2 S7d-keys.8)
/// calls this once per known ring peer. The call may recover the transport
/// route before returning a reply for verification.
///
/// # Errors
/// Encoding failed, the call failed, the reply is not a frontier response
/// (a refusal is a NAK), or a head does not hold.
pub async fn request_frontier<D, F>(
    deps: &D,
    community: &str,
    me: &SigningKey,
    call: impl FnOnce(Vec<u8>) -> F,
) -> Result<PeerFrontier, GovernanceRuntimeError>
where
    D: GovernanceRuntimeDeps,
    F: Future<Output = Result<Vec<u8>, GovernanceRuntimeError>>,
{
    let request = FrontierRequest {
        community: community.to_owned(),
        requester: me.verifying_key().to_bytes(),
    }
    .encode(me)
    .map_err(|e| encoding("frontier request", e))?;
    let envelope = encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::FrontierRequest { request },
    ))
    .map_err(|e| encoding("frontier envelope", e))?;
    let reply = call(envelope).await?;
    let frontier = match try_decode_community_envelope(&reply)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::FrontierResponse { frontier }) => {
            decode_frontier_response(&frontier).map_err(|e| encoding("frontier response", e))?
        }
        _ => {
            return Err(GovernanceRuntimeError::Adapter(
                "the reply is not a frontierResponse".into(),
            ))
        }
    };
    verify_frontier_heads(deps, community, frontier).await
}

#[cfg(test)]
#[path = "frontier_exchange_tests.rs"]
mod tests;
