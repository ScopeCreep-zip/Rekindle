//! Peer route import. Our own routes are allocated, healed and released by
//! `OwnRoutes` (plan C7.9d, `TransportNode::own_routes`) and republished by
//! the route publisher; resume publishes the blob at unlock.

use tracing::{debug, warn};

use super::node::TransportNode;
use super::peer_registry::PeerTarget;
use crate::error::Result;

/// Import a remote peer's route blob, returning a `PeerTarget` handle.
pub fn import_peer_route(node: &TransportNode, route_blob: &[u8]) -> Result<PeerTarget> {
    debug!(blob_bytes = route_blob.len(), "route: importing peer");
    let result = node.import_route(route_blob);
    if let Err(ref e) = result {
        warn!(error = %e, "route: peer import failed");
    }
    result
}
