//! A checkpoint's state snapshot (plan E3.2 S7c.3, "State snapshot"): the
//! bytes of `canonical_state ‖ canonical_cut` at its cut, the preimage of
//! its `stateRoot`. A member serves them in chunks of at most
//! [`SNAPSHOT_CHUNK_MAX`] B ([`answer_state_request`]; the Host admits the
//! requester and rate-limits it first), and a reader fetches them from a
//! ring peer, checks the reassembled bytes against `stateRoot` before
//! decoding and asks the next ring peer on a mismatch
//! ([`fetch_state_snapshot`]), refusing at the first chunk a total other
//! than the checkpoint's endorsed `stateSize` (S7c.4a: the state has no
//! size bound of its own, so the checkpoint commits to the exact one). With
//! no peer that matches the caller walks the pre-cut ops, the full replay.

use std::future::Future;

use rekindle_codec::capnp_envelope::{encode_community_envelope, try_decode_community_envelope};
use rekindle_codec::checkpoint_state::{
    decode_checkpoint_state_response, CheckpointStateRequest, CheckpointStateResponse,
    PreJoinRequest, SNAPSHOT_CHUNK_MAX,
};
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::checkpoint::{checkpoint_digest, state_preimage, state_root_of};
use rekindle_governance::merge::merge_cut;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{Checkpoint, GovernanceEntry};

use crate::deps::GovernanceRuntimeDeps;
use crate::op_log::{causal_past, verify_held_with};
use crate::GovernanceRuntimeError;

fn no_state(why: impl Into<String>) -> GovernanceRuntimeError {
    GovernanceRuntimeError::NoCheckpointState(why.into())
}

/// The snapshot at the cut of the checkpoint with this `height` and
/// `digest` among the ops we hold: the merge of its cut's causal closure
/// (`merge_cut`), served only when it reproduces the checkpoint's
/// `stateRoot`, since bytes that miss it are refused by every reader. A
/// usable hint's snapshot is kept on its `governance_checkpoints` row and
/// read from there while the row stays usable.
///
/// The kept row is read first, before the op set is loaded or verified
/// (S7c.8): a snapshot is written to it only below, after our merge of the
/// verified cut reproduced the `stateRoot` the row's digest commits to,
/// and the row is read only while the last rebuild, over the verified op
/// set (`checkpoint_rows::checkpoint_facts`), holds it `usable`. So each of
/// a snapshot's chunk requests after the first costs one row read, not a
/// load and verification of every held op. Synapse serves `/state_ids`
/// the same way: from the state group persisted when the event's state was
/// resolved, never by re-running auth over the room's events per request.
///
/// # Errors
/// No cert is pinned, the op store failed, or no snapshot is held
/// ([`GovernanceRuntimeError::NoCheckpointState`]).
pub async fn snapshot_at<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    height: u64,
    digest: &[u8; 32],
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    if let Some(kept) = deps
        .op_store()
        .usable_checkpoint_snapshot(community_id, height, *digest)
        .await?
    {
        return Ok(kept);
    }
    let held = deps.op_store().governance_ops(community_id).await?;
    let set = verify_held_with(deps, community_id, &held);
    let checkpoint = set
        .ops
        .iter()
        .find_map(|op| match op.body() {
            GovernanceEntry::Checkpoint(c)
                if c.height == height && checkpoint_digest(c) == *digest =>
            {
                Some(c.clone())
            }
            _ => None,
        })
        .ok_or_else(|| no_state(format!("no checkpoint at height {height} with that digest")))?;
    let (state, cut) = merge_cut(&cert, &causal_past(&set.ops, &checkpoint.cut));
    let preimage = state_preimage(&state, &cut);
    if state_root_of(&preimage) != checkpoint.state_root {
        return Err(no_state("our merge of the cut misses its stateRoot"));
    }
    deps.op_store()
        .keep_checkpoint_snapshot(community_id, height, *digest, preimage.clone())
        .await?;
    Ok(preimage)
}

/// The chunk of `snapshot` at `offset`: at most [`SNAPSHOT_CHUNK_MAX`] B.
///
/// # Errors
/// The snapshot is over `u32::MAX` B, or `offset` is past its end
/// ([`GovernanceRuntimeError::NoCheckpointState`]).
pub fn chunk_at(
    snapshot: &[u8],
    offset: u32,
) -> Result<CheckpointStateResponse, GovernanceRuntimeError> {
    let total = u32::try_from(snapshot.len()).map_err(|_| no_state("snapshot over u32::MAX"))?;
    let start = offset as usize;
    if start > snapshot.len() || (start == snapshot.len() && start != 0) {
        return Err(no_state(format!(
            "offset {offset} is past the {total} B snapshot"
        )));
    }
    let end = snapshot.len().min(start + SNAPSHOT_CHUNK_MAX);
    Ok(CheckpointStateResponse {
        total,
        bytes: snapshot[start..end].to_vec(),
    })
}

/// Answer an admitted member's verified `request`: the encoded
/// `checkpointStateResponse` envelope.
///
/// # Errors
/// The community is partial (S7c.8 (1)), no snapshot is held at that
/// checkpoint or offset, or encoding failed.
pub async fn answer_state_request<D: GovernanceRuntimeDeps>(
    deps: &D,
    request: &CheckpointStateRequest,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    crate::partial::refuse_while_partial(deps, &request.community).await?;
    let snapshot = snapshot_at(deps, &request.community, request.height, &request.digest).await?;
    let response = chunk_at(&snapshot, request.offset)?
        .encode()
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("checkpoint state response: {e}")))?;
    encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::CheckpointStateResponse { response },
    ))
    .map_err(|e| GovernanceRuntimeError::Encoding(format!("checkpoint state envelope: {e}")))
}

/// Reassemble one peer's `total`-byte snapshot through `ask(offset)`:
/// `None` when an answer fails, claims another total, or is not the exact
/// chunk the offset calls for (every chunk but the last is a full
/// [`SNAPSHOT_CHUNK_MAX`]), so a peer claiming more than the endorsed size
/// is dropped at its first chunk.
async fn reassemble<F, Fut>(total: u32, mut ask: F) -> Option<Vec<u8>>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<CheckpointStateResponse, GovernanceRuntimeError>>,
{
    let total = total as usize;
    let mut chunk = ask(0).await.ok()?;
    let mut bytes = Vec::new();
    loop {
        let expected = (total - bytes.len()).min(SNAPSHOT_CHUNK_MAX);
        if chunk.total as usize != total || chunk.bytes.len() != expected {
            return None;
        }
        bytes.extend_from_slice(&chunk.bytes);
        if bytes.len() == total {
            return Some(bytes);
        }
        chunk = ask(u32::try_from(bytes.len()).ok()?).await.ok()?;
    }
}

/// The first of `peers` (ring order) whose reassembled snapshot of
/// `state_size` bytes hashes to `state_root`, asking peer `i` for the chunk
/// at an offset through `ask(i, offset)`; a peer whose bytes miss it is
/// refused and the next is asked. `None` when no peer matches: the caller
/// walks the pre-cut ops.
pub async fn first_matching<F, Fut>(
    peers: usize,
    (state_root, state_size): (&[u8; 32], u32),
    mut ask: F,
) -> Option<Vec<u8>>
where
    F: FnMut(usize, u32) -> Fut,
    Fut: Future<Output = Result<CheckpointStateResponse, GovernanceRuntimeError>>,
{
    for peer in 0..peers {
        match reassemble(state_size, |offset| ask(peer, offset)).await {
            Some(bytes) if state_root_of(&bytes) == *state_root => return Some(bytes),
            Some(_) => tracing::warn!(peer, "checkpoint state snapshot misses the stateRoot"),
            None => tracing::debug!(peer, "checkpoint state snapshot not served"),
        }
    }
    None
}

/// Fetch the snapshot of `checkpoint` from the ring peers at
/// `peer_routes`, nearest first, asking as `me` (our pseudonym in
/// `community_id`), a member, or, before the join, a joiner holding the
/// invite `invite_code` (plan E3.2 S7c.5; each request tagged under the
/// code, S7c.8 (2)): its
/// `stateSize` bytes that hash to its `stateRoot`, or `None` when no peer
/// matches.
///
/// # Errors
/// Our request could not be encoded.
pub async fn fetch_state_snapshot<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    me: &SigningKey,
    checkpoint: &Checkpoint,
    peer_routes: &[Vec<u8>],
    invite_code: Option<&str>,
) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
    let (height, digest) = (checkpoint.height, checkpoint_digest(checkpoint));
    let request = |offset| {
        let request = CheckpointStateRequest {
            community: community_id.to_owned(),
            height,
            digest,
            offset,
            requester: me.verifying_key().to_bytes(),
            invite: None,
        };
        let request = match invite_code {
            Some(code) => request.with_invite_code(code),
            None => request,
        };
        let encoded = request.encode(me).map_err(|e| {
            GovernanceRuntimeError::Encoding(format!("checkpoint state request: {e}"))
        })?;
        encode_community_envelope(&CommunityEnvelope::Control(
            ControlPayload::CheckpointStateRequest { request: encoded },
        ))
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("checkpoint state envelope: {e}")))
    };
    request(0)?;
    let expected = (&checkpoint.state_root, checkpoint.state_size);
    Ok(first_matching(peer_routes.len(), expected, |peer, offset| {
        let call = request(offset);
        async move {
            let reply = deps.app_call_peer(&peer_routes[peer], call?).await?;
            match try_decode_community_envelope(&reply) {
                Ok(Some(CommunityEnvelope::Control(ControlPayload::CheckpointStateResponse {
                    response,
                }))) => decode_checkpoint_state_response(&response).map_err(|e| {
                    GovernanceRuntimeError::Encoding(format!("checkpoint state response: {e}"))
                }),
                _ => Err(no_state("the reply is not a checkpointStateResponse")),
            }
        }
    })
    .await)
}

#[cfg(test)]
#[path = "checkpoint_state_tests.rs"]
mod tests;
