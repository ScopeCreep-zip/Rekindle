//! A peer's route, from the one peer-route cache (`PeerRegistry`, plan
//! E2.1) or, when it has none, from the peer's profile record.

use rekindle_records::port::PeerTarget;

use crate::broadcast::node::TransportNode;
use crate::operations::friend::{state, FriendHost};

/// The peer's cached route blob while it is fresh; a stale one is dropped.
#[must_use]
pub fn cached(node: &TransportNode, peer: &str) -> Option<Vec<u8>> {
    let peers = node.peers();
    let mut registry = peers.write();
    if let Some(blob) = registry.get_route(peer) {
        return Some(blob.to_vec());
    }
    registry.invalidate_route(peer);
    None
}

/// Cache a peer's route blob (from a request, an accept, an invite, their
/// profile or a relay).
pub fn cache(node: &TransportNode, peer: &str, blob: Vec<u8>) {
    node.peers().write().cache_route(peer, blob);
}

/// Forget a peer's cached route blob.
pub fn invalidate(node: &TransportNode, peer: &str) {
    node.peers().write().invalidate_route(peer);
}

/// Import `blob` through the node's one importer; a blob that does not
/// import is the peer's no longer, so it is forgotten.
pub(crate) fn import(node: &TransportNode, peer: &str, blob: &[u8]) -> Option<PeerTarget> {
    match node.routes().import(blob) {
        Ok(target) => Some(target),
        Err(e) => {
            tracing::debug!(to = %peer, error = %e, blob_len = blob.len(),
                "route import failed — invalidating cached route");
            invalidate(node, peer);
            None
        }
    }
}

/// The peer's imported cached route, if they have a fresh one.
pub(crate) fn cached_target(node: &TransportNode, peer: &str) -> Option<PeerTarget> {
    let blob = cached(node, peer)?;
    import(node, peer, &blob)
}

/// Read a fresh route blob from the peer's profile record (subkey 6): a
/// friend's, or a group member's from the roster we hold; and
/// cache it.
pub async fn fetch_from_profile(host: &dyn FriendHost, peer: &str) -> Option<Vec<u8>> {
    let node = host.node().ok()?;
    let profile_key = profile_key(host, peer).await?;
    let records = node.require_records().ok()?;
    let blob = rekindle_protocol::dht::profile::read_profile_subkey(
        records.as_ref(),
        &profile_key,
        rekindle_protocol::dht::profile::SUBKEY_ROUTE_BLOB,
        true,
    )
    .await
    .ok()??;
    if blob.is_empty() {
        return None;
    }
    // A route we suspected stays out of use for its failure cache, unless
    // its probe answers (plan RH1r-c S2).
    if node.liveness().failure_cached(peer, &blob) {
        tracing::debug!(peer = %peer, "profile route is the one we suspect; not using it");
        return None;
    }
    cache(&node, peer, blob.clone());
    tracing::debug!(peer = %peer, "fetched fresh route blob from DHT inline");
    Some(blob)
}

/// The profile record a peer's route is published on: a friend's, else
/// the roster's for a group member who is not our friend (plan E2.6).
async fn profile_key(host: &dyn FriendHost, peer: &str) -> Option<String> {
    match state::friend(host, peer).and_then(|f| f.dht_record_key) {
        Some(key) => Some(key),
        None => crate::operations::group_dm::member_profile(host, peer).await,
    }
}

/// The peer's route blob and its record seq, read from the network and not
/// cached (plan RH1r-c S2: the caller takes it only under a higher seq).
pub(crate) async fn read_profile_route(
    host: &dyn FriendHost,
    peer: &str,
) -> Option<(Vec<u8>, u32)> {
    let profile_key = profile_key(host, peer).await?;
    let records = host.node().ok()?.require_records().ok()?;
    rekindle_protocol::dht::profile::read_profile_subkey_seq(
        records.as_ref(),
        &profile_key,
        rekindle_protocol::dht::profile::SUBKEY_ROUTE_BLOB,
    )
    .await
    .ok()?
    .filter(|(blob, _)| !blob.is_empty())
}

/// Read the friend's current route from their mailbox record and cache it.
pub async fn fetch_from_mailbox(host: &dyn FriendHost, peer: &str) -> Option<Vec<u8>> {
    let mailbox_key = state::friend(host, peer)?.mailbox_dht_key?;
    let node = host.node().ok()?;
    let records = node.require_records().ok()?;
    match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(records.as_ref(), &mailbox_key)
        .await
    {
        Ok(Some(blob)) if !blob.is_empty() => {
            cache(&node, peer, blob.clone());
            Some(blob)
        }
        Ok(_) => None,
        Err(error) => {
            tracing::trace!(to = %peer, %error, "failed to read mailbox");
            None
        }
    }
}

/// `peer` is reachable again (a new route on their watched profile, or
/// their accept opened our session): our rows to them that wait out a
/// backoff are attempted now, not at their next due time (live finding
/// L-FR2; the event drives it, no poll, D4). On the session scope, for a
/// caller inside an apply or a watch callback.
pub fn reachable_again(host: &dyn FriendHost, peer: &str, why: &'static str) {
    let Ok(outbox) = host.outbox() else {
        return;
    };
    let peer = peer.to_string();
    host.scope()
        .spawn_or_drop("bring a peer's rows forward", async move {
            match outbox.bring_forward(&peer).await {
                Ok(moved) => tracing::debug!(moved, why, "waiting rows brought forward"),
                Err(e) => tracing::warn!(error = %e, why, "waiting rows not brought forward"),
            }
        });
}
