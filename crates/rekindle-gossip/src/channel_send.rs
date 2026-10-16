//! Channel-scoped sends: an envelope to exactly the voice/video channel
//! roster, signed like the mesh path but with `ttl = 0` (architecture
//! §10.6). Split from `mesh_broadcast` (plan C7.27 added the awaited
//! variant the session end uses).

use std::sync::Arc;

use rekindle_codec::capnp_envelope::encode_signed_envelope;
use rekindle_codec::community::envelope::CommunityEnvelope;

use crate::deps::{GossipDeps, GossipError, PeerInfo};
use crate::mesh_broadcast::{send_to_one_peer, sign_and_record};

/// Directed, channel-scoped variant of [`crate::send_to_mesh`]. Signs the
/// envelope exactly like the mesh path but forces `ttl = 0` and fans
/// out to exactly the given peers (the voice/video channel roster)
/// instead of the community gossip overlay. Architecture §10.6 —
/// media and its per-stream control traffic flow only among the
/// members actively in the channel, so there is no reliability
/// ranking, no fan-out degree cap, and no pending-mesh queue: a
/// frame with no recipients is stale by the next frame interval and
/// is dropped, not deferred.
pub async fn send_to_channel_peers<D: GossipDeps>(
    deps: Arc<D>,
    community_id: &str,
    envelope: &CommunityEnvelope,
    peers: Vec<PeerInfo>,
) -> Result<(), GossipError> {
    let Some(signed_bytes) = sign_for_channel(&deps, community_id, envelope, &peers)? else {
        return Ok(());
    };
    let cid_owner = community_id.to_string();
    // Dropping the supervisor (session end) drops the JoinSet, which
    // aborts every per-peer send.
    let scope = deps.scope();
    scope.spawn_or_drop("channel peer fan-out", async move {
        fan_out(deps, cid_owner, peers, signed_bytes).await;
    });
    Ok(())
}

/// [`send_to_channel_peers`], awaited: returns once every per-peer send
/// has been handed to the transport, rather than spawning onto the
/// session scope. For a message that must leave while the session is
/// being torn down, whose scope is about to be cancelled: the
/// `VoiceLeave` sent when the user quits or logs out mid-call (plan
/// C7.27).
pub async fn send_to_channel_peers_now<D: GossipDeps>(
    deps: Arc<D>,
    community_id: &str,
    envelope: &CommunityEnvelope,
    peers: Vec<PeerInfo>,
) -> Result<(), GossipError> {
    let Some(signed_bytes) = sign_for_channel(&deps, community_id, envelope, &peers)? else {
        return Ok(());
    };
    fan_out(deps, community_id.to_string(), peers, signed_bytes).await;
    Ok(())
}

/// Sign a channel-scoped envelope (`ttl = 0`: honest receivers never
/// gossip-forward it beyond the roster the sender addressed). `None` when
/// there is nobody to send to.
fn sign_for_channel<D: GossipDeps>(
    deps: &Arc<D>,
    community_id: &str,
    envelope: &CommunityEnvelope,
    peers: &[PeerInfo],
) -> Result<Option<Vec<u8>>, GossipError> {
    if peers.is_empty() {
        tracing::debug!(
            community = %community_id,
            "send_to_channel_peers: no channel peers — dropping",
        );
        return Ok(None);
    }
    let mut signed = sign_and_record(deps, community_id, envelope)?;
    signed.ttl = 0;
    Ok(Some(encode_signed_envelope(&signed)))
}

/// Send `data` to every peer concurrently and wait for all of them.
async fn fan_out<D: GossipDeps>(
    deps: Arc<D>,
    community_id: String,
    peers: Vec<PeerInfo>,
    data: Vec<u8>,
) {
    let mut set = tokio::task::JoinSet::new();
    for peer in peers {
        let deps = Arc::clone(&deps);
        let cid = community_id.clone();
        let data = data.clone();
        set.spawn(async move {
            send_to_one_peer(deps, cid, peer, data, None).await;
        });
    }
    while set.join_next().await.is_some() {}
}
