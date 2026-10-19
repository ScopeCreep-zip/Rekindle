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

/// Read a fresh route blob from the friend's profile record (subkey 6) and
/// cache it.
pub async fn fetch_from_profile(host: &dyn FriendHost, peer: &str) -> Option<Vec<u8>> {
    let profile_key = state::friend(host, peer)?.dht_record_key?;
    let node = host.node().ok()?;
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
    cache(&node, peer, blob.clone());
    tracing::debug!(peer = %peer, "fetched fresh route blob from DHT inline");
    Some(blob)
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
