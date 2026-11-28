//! Phase 20 REDO — full mesh-broadcast orchestrators.
//!
//! Pre-port these bodies lived in `src-tauri/services/community/gossip.rs`
//! and read `state.communities` / `state.identity_secret` / etc. directly.
//! Here they parameterise over `GossipDeps` so the entire pipeline (sign
//! + dedup + peer-select + supervised per-peer fan-out
//! with route re-resolution) is testable against a mock and free of
//! `veilid-core` / `tauri` / `rusqlite`.

use std::sync::Arc;

use tracing::Instrument as _;

use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{self, CommunityEnvelope, SignedEnvelope};
use rekindle_codec::dedup::message_dedup_key;
use rekindle_codec::ingress::gossip_datagram;

use crate::deps::{GossipDeps, GossipError, PeerInfo};
use crate::mesh::fanout_degree;
use crate::peer_select::sort_peers_by_reliability;

/// Maximum number of broadcasts queued per community when fan-out
/// finds zero peers. Bounded so an offline burst doesn't OOM the
/// process. The adapter enforces this cap inside
/// `GossipDeps::enqueue_pending_mesh`; the constant lives here so
/// tests + tracing can reference it.
pub const MAX_PENDING_MESH: usize = 100;

/// Sign one envelope as our community pseudonym under a fresh message id,
/// and record that id in the dedup cache (so a relayed copy coming back
/// to us is suppressed).
///
/// The id is new on every call: a deliberate re-send of identical
/// content (the voice roster reconcile, a retry) is a new message that
/// flood suppression must not drop; receivers are idempotent.
///
/// Every gossip send does exactly these things before it differs.
/// `send_to_mesh` and `send_to_channel_peers` had them written out
/// twice, identically — a shared *prefix* rather than a shared body,
/// which is why `cargo xtask check-duplicate-bodies` cannot see it: the
/// two functions diverge in their tails (overlay fan-out versus a fixed
/// roster) so their normalised bodies differ.
///
/// Returns the signed envelope for the caller to fan out. The caller
/// owns the fan-out because that is the only part that legitimately
/// differs — `send_to_channel_peers` additionally forces `ttl = 0` so
/// honest receivers never gossip-forward channel media past the roster
/// the sender addressed (architecture §10.6).
pub(crate) fn sign_and_record<D: GossipDeps>(
    deps: &Arc<D>,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<SignedEnvelope, GossipError> {
    let my_pseudonym_key = deps.my_pseudonym_key(community_id);
    let identity_secret = deps
        .identity_secret()
        .ok_or(GossipError::IdentityNotLoaded)?;

    let signing_key = rekindle_crypto::group::pseudonym::derive_community_pseudonym(
        &identity_secret,
        community_id,
    );
    let envelope_bytes = encode_community_envelope(envelope)?;
    let message_id = rekindle_utils::random::id_bytes_16_from(deps.rng().as_ref());
    let signed = envelope::sign_envelope(
        &signing_key,
        community_id,
        &my_pseudonym_key,
        message_id,
        &envelope_bytes,
    );

    deps.check_and_insert_dedup(community_id, &my_pseudonym_key, &message_dedup_key(&signed));

    Ok(signed)
}

/// Encode + sign the envelope, insert the dedup key, then fan out via
/// `send_to_mesh_raw`. The envelope's Lamport timestamp was taken from
/// the community clock when it was built — that is the send's one tick.
///
/// Synchronous up to the fan-out spawn: the peers are the overlay's at the
/// call, so a caller that changes the overlay next (a removal's install
/// prunes its target) has already chosen whom the envelope goes to.
pub fn send_to_mesh<D: GossipDeps>(
    deps: Arc<D>,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), GossipError> {
    let signed = sign_and_record(&deps, community_id, envelope)?;
    send_to_mesh_raw(deps, community_id, signed)
}

/// Fan-out a pre-signed envelope. Idempotent + crash-safe by design:
/// if `current_peers` returns zero or no overlay, the envelope is queued for the
/// next presence-poll cycle instead of being dropped (architecture
/// A1/P4.1). On a non-empty peer list, ranks by reliability +
/// applies the architecture §3 fan-out degree + spawns a single
/// supervising tokio task with a `JoinSet` so per-peer tasks are
/// owned (M9.6, not orphaned).
pub fn send_to_mesh_raw<D: GossipDeps>(
    deps: Arc<D>,
    community_id: &str,
    signed: SignedEnvelope,
) -> Result<(), GossipError> {
    // The `'G'` gossip datagram (§6 ingress ledger), as every gossip send.
    let signed_bytes = gossip_datagram(&signed)?;

    // No overlay yet (a community just joined, before its first presence
    // poll builds one) holds the send like an empty one: the poll that
    // lands peers drains it (architecture A1/P4.1).
    let peers = deps.current_peers(community_id).unwrap_or_default();
    if peers.is_empty() {
        deps.enqueue_pending_mesh(community_id, signed);
        tracing::info!(
            community = %community_id,
            "send_to_mesh_raw: peers empty, queued for later drain",
        );
        return Ok(());
    }

    let selected = select_fanout(&*deps, community_id, peers);
    tracing::info!(
        community = %community_id,
        peer_count = selected.len(),
        "send_to_mesh_raw: sending to gossip fan-out",
    );
    let cid_owner = community_id.to_string();
    deps.scope().spawn_or_drop("mesh fan-out", async move {
        fan_out(deps, cid_owner, selected, signed_bytes).await;
    });
    Ok(())
}

/// Sign `envelope` and send it to the fan-out now, awaiting every peer's
/// send: the send at a session's end (the quit's beat hint, plan E3.4), when
/// the session's scope is already closed and [`send_to_mesh`]'s spawned
/// fan-out would be dropped. Nothing is queued without peers: no session
/// follows to drain it. Returns the number of peers sent to.
///
/// # Errors
/// The envelope could not be signed or encoded.
pub async fn send_to_mesh_awaited<D: GossipDeps>(
    deps: Arc<D>,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<usize, GossipError> {
    let signed = sign_and_record(&deps, community_id, envelope)?;
    let peers = deps.current_peers(community_id).unwrap_or_default();
    let selected = select_fanout(&*deps, community_id, peers);
    let sent = selected.len();
    fan_out(
        deps,
        community_id.to_owned(),
        selected,
        gossip_datagram(&signed)?,
    )
    .await;
    Ok(sent)
}

/// The architecture §3 fan-out of `peers`, the most reliable first.
fn select_fanout<D: GossipDeps>(
    deps: &D,
    community_id: &str,
    peers: Vec<PeerInfo>,
) -> Vec<PeerInfo> {
    let scores = deps.peer_reliability_scores(community_id);
    let scored = sort_peers_by_reliability(peers, &scores);
    let degree = fanout_degree(scored.len());
    scored.into_iter().take(degree).collect()
}

/// Send `data` to each of `selected`, one owned task per peer (M9.6), and
/// wait for them all.
async fn fan_out<D: GossipDeps>(
    deps: Arc<D>,
    community_id: String,
    selected: Vec<PeerInfo>,
    data: Vec<u8>,
) {
    let mut set = tokio::task::JoinSet::new();
    for peer in selected {
        let deps = Arc::clone(&deps);
        let cid = community_id.clone();
        let data = data.clone();
        set.spawn(async move { send_to_one_peer(deps, cid, peer, data).await }.in_current_span());
    }
    while set.join_next().await.is_some() {}
}

/// One peer send + route re-resolve + retry. Records the peer's
/// reliability on the first attempt's outcome.
pub(crate) async fn send_to_one_peer<D: GossipDeps>(
    deps: Arc<D>,
    community_id: String,
    peer: PeerInfo,
    data: Vec<u8>,
) {
    let first = if peer.route_blob.is_empty() {
        // Peer published presence before its route allocation finished
        // (the registry warns "our_route_blob is None" on that path).
        // An empty blob can never import — skip straight to re-resolve.
        Err("empty route blob in peer cache".to_string())
    } else {
        deps.send_app_message(&peer.route_blob, data.clone()).await
    };

    if first.is_ok() {
        deps.record_peer_reliability(&community_id, &peer.pseudonym_key, true);
        return;
    }

    deps.record_peer_reliability(&community_id, &peer.pseudonym_key, false);

    // Single-flight: one task leads the DHT re-resolution per
    // (community, peer); concurrent failures (a queued-broadcast drain
    // fires dozens at once) wait for the leader's write-back and retry
    // against the refreshed overlay route instead of issuing their own
    // DHT lookups.
    let lead = deps
        .resolve_gate()
        .try_lead(&community_id, &peer.pseudonym_key);
    let fresh_blob = if let Some(_lead_guard) = lead {
        tracing::info!(
            community = %community_id,
            peer = %peer.pseudonym_key,
            "route stale, attempting DHT re-resolve",
        );
        let resolved = deps
            .resolve_peer_route_from_dht(&community_id, &peer.pseudonym_key)
            .await;
        if let Some(ref blob) = resolved {
            let status = deps
                .online_member_status(&community_id, &peer.pseudonym_key)
                .unwrap_or_else(|| "online".to_string());
            deps.update_peer_route(&community_id, &peer.pseudonym_key, &status, blob.clone());
        }
        resolved
        // _lead_guard drops here — waiters proceed against the
        // overlay route written back above.
    } else {
        deps.resolve_gate()
            .wait(&community_id, &peer.pseudonym_key)
            .await;
        // The leader's write-back (if any) is in the overlay now.
        deps.current_peers(&community_id)
            .into_iter()
            .flatten()
            .find(|p| p.pseudonym_key == peer.pseudonym_key)
            .map(|p| p.route_blob)
            .filter(|blob| !blob.is_empty() && *blob != peer.route_blob)
    };

    let Some(fresh_blob) = fresh_blob else {
        tracing::warn!(
            community = %community_id,
            peer = %peer.pseudonym_key,
            "no fresh route found in DHT",
        );
        return;
    };

    if let Err(error) = deps.send_app_message(&fresh_blob, data).await {
        tracing::warn!(
            community = %community_id,
            peer = %peer.pseudonym_key,
            %error,
            "re-resolved route still failed",
        );
    }
}

#[cfg(test)]
#[path = "mesh_broadcast_tests.rs"]
mod tests;
