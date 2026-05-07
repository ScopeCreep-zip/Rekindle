//! Internal broadcast helpers shared by all gossip submodules.

use std::collections::HashMap;

use parking_lot::RwLock;
use tracing::{debug, trace, warn};

use crate::broadcast::node::TransportNode;
use crate::broadcast::send::BroadcastReport;
use crate::crypto::envelope;
use crate::gossip::GossipMesh;
use crate::payload::gossip::GossipPayload;

/// Type alias for the community gossip mesh map.
pub type MeshMap = HashMap<String, GossipMesh>;

/// Default TTL for gossip broadcasts.
const DEFAULT_TTL: u8 = 3;

/// Build a signed gossip envelope and fan out to mesh peers.
pub async fn build_sign_send(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender_pseudonym: &str,
    signing_key: &[u8; 32],
    payload: GossipPayload,
) -> BroadcastReport {
    let payload_bytes = match postcard::to_stdvec(&payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "gossip broadcast: payload serialization failed");
            return BroadcastReport { delivered: 0, failures: vec![("*".into(), format!("serialize: {e}"))] };
        }
    };

    let (lamport_ts, peer_targets) = {
        let mut guard = meshes.write();
        let Some(mesh) = guard.get_mut(community_id) else {
            debug!(community_id, "gossip broadcast: no mesh for community");
            return BroadcastReport::default();
        };
        let ts = mesh.clock.increment();
        let targets: Vec<(String, Vec<u8>)> = mesh.peers.iter()
            .map(|(k, m)| (k.clone(), m.route_blob.clone()))
            .collect();
        (ts, targets)
    };

    if peer_targets.is_empty() {
        trace!(community_id, "gossip broadcast: no peers in mesh");
        return BroadcastReport::default();
    }

    let signing = ed25519_dalek::SigningKey::from_bytes(signing_key);
    let envelope = envelope::sign_gossip_envelope(
        &signing, community_id, sender_pseudonym,
        &payload_bytes, DEFAULT_TTL, lamport_ts,
    );

    let sender = node.sender();
    let mut targets_with_routes = Vec::with_capacity(peer_targets.len());
    for (key, blob) in &peer_targets {
        match node.import_route(blob) {
            Ok(target) => targets_with_routes.push((key.clone(), target)),
            Err(e) => debug!(peer = %key, error = %e, "gossip broadcast: route import failed"),
        }
    }

    sender.broadcast_gossip_parallel(&targets_with_routes, &envelope, 16).await
}

/// Shortcut for broadcasting a ControlPayload wrapped in GossipPayload::Control.
pub(super) async fn control(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, signing_key: &[u8; 32],
    ctrl: crate::payload::gossip::ControlPayload,
) -> BroadcastReport {
    build_sign_send(node, meshes, community_id, sender, signing_key,
        GossipPayload::Control(ctrl)).await
}

/// Send a signed gossip envelope directly to a single target via their route.
///
/// Used for point-to-point notifications (e.g., JoinAccepted to a specific joiner)
/// where the target is NOT in the gossip mesh.
pub async fn send_direct(
    node: &TransportNode,
    community_id: &str,
    sender_pseudonym: &str,
    signing_key: &[u8; 32],
    payload: GossipPayload,
    target_key: &str,
    target_route_blob: &[u8],
) -> BroadcastReport {
    let payload_bytes = match postcard::to_stdvec(&payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "direct gossip: payload serialization failed");
            return BroadcastReport { delivered: 0, failures: vec![("*".into(), format!("serialize: {e}"))] };
        }
    };

    let signing = ed25519_dalek::SigningKey::from_bytes(signing_key);
    let envelope = envelope::sign_gossip_envelope(
        &signing, community_id, sender_pseudonym,
        &payload_bytes, 0, 0,
    );

    let target = match node.import_route(target_route_blob) {
        Ok(t) => t,
        Err(e) => {
            debug!(target = target_key, error = %e, "direct gossip: route import failed");
            return BroadcastReport { delivered: 0, failures: vec![(target_key.into(), format!("{e}"))] };
        }
    };

    node.sender().broadcast_gossip(&[(target_key.to_string(), target)], &envelope).await
}
