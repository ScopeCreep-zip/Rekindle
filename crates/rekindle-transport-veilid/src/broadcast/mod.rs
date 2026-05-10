//! Consolidated outbound module — the sole Veilid boundary for all outgoing data.
//!
//! Every way data leaves the node to Veilid lives here. No other module
//! in the workspace imports `veilid_core`. This is the strict outbound boundary.
//!
//! # Submodules
//!
//! ## Veilid lifecycle & infrastructure
//! - `node` — VeilidAPI lifecycle (startup, shutdown, attach, detach, RoutingContext)
//! - `send` — app_message / app_call outbound wrappers (opaque bytes only)
//! - `peer_route` — route allocation, import, release (RouteManager)
//! - `peer_registry` — peer route caching and circuit breaking (PeerRegistry)
//! - `dht/` — all DHT record CRUD (create, open, close, get, set, watch, inspect)
//!
//! ## Broadcast helpers
//! - `dht_writes` — thin primitive wrappers over dht/ for TransportNode callers
//! - `rpc` — request-response RPC calls (opaque bytes)
//! - `voice` — voice packet send (opaque bytes)
//! - `route` — route lifecycle convenience (allocate, refresh, publish)

// Veilid infrastructure (imports veilid_core)
pub mod node;
pub mod send;
pub mod peer_route;
pub mod peer_registry;
pub mod dht;

// Broadcast helpers
pub mod dht_writes;
pub mod route;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{debug, trace};

use crate::gossip::GossipMesh;
use node::TransportNode;
use send::BroadcastReport;

/// Rate limiter for outbound gossip, keyed by a string identifier.
#[derive(Debug, Default)]
pub struct OutboundRateLimiter {
    last_sent: HashMap<String, std::time::Instant>,
}

impl OutboundRateLimiter {
    pub fn check(&mut self, key: &str, min_interval: std::time::Duration) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.last_sent.get(key) {
            if now.duration_since(*last) < min_interval {
                return false;
            }
        }
        self.last_sent.insert(key.to_string(), now);
        true
    }

    pub fn remove_community(&mut self, community: &str) {
        self.last_sent.retain(|k, _| !k.starts_with(community));
    }
}

/// Centralized outbound broadcast manager.
///
/// Holds the TransportNode (Veilid API) and gossip mesh state.
/// Does NOT hold Session or MekCache — those are application concerns
/// managed by rekindle-chat. Transport sends opaque bytes.
pub struct BroadcastManager {
    pub(crate) node: Arc<TransportNode>,
    pub(crate) meshes: Arc<RwLock<HashMap<String, GossipMesh>>>,
    pub(crate) rate_limiter: RwLock<OutboundRateLimiter>,
}

impl BroadcastManager {
    pub fn new(node: Arc<TransportNode>) -> Self {
        Self {
            node,
            meshes: Arc::new(RwLock::new(HashMap::new())),
            rate_limiter: RwLock::new(OutboundRateLimiter::default()),
        }
    }

    pub fn register_mesh(&self, community_id: &str) {
        tracing::info!(community_id, "broadcast: registering gossip mesh");
        self.meshes.write().entry(community_id.to_string())
            .or_insert_with(|| GossipMesh::new(community_id.to_string()));
    }

    pub fn deregister_mesh(&self, community_id: &str) {
        tracing::info!(community_id, "broadcast: deregistering gossip mesh");
        self.meshes.write().remove(community_id);
        self.rate_limiter.write().remove_community(community_id);
    }

    pub fn node(&self) -> &TransportNode { &self.node }
    pub fn meshes(&self) -> &Arc<RwLock<HashMap<String, GossipMesh>>> { &self.meshes }

    /// Fan out pre-signed, pre-framed bytes to all mesh peers for a community.
    ///
    /// Chat has already serialized, signed, and framed the gossip envelope.
    /// Transport resolves mesh peers, imports routes, and sends raw bytes.
    pub async fn broadcast_to_mesh(
        &self,
        community_id: &str,
        data: &[u8],
    ) -> BroadcastReport {
        let peer_targets = {
            let guard = self.meshes.read();
            let Some(mesh) = guard.get(community_id) else {
                debug!(community_id, "broadcast: no mesh for community");
                return BroadcastReport::default();
            };
            mesh.peers
                .iter()
                .map(|(k, m)| (k.clone(), m.route_blob.clone()))
                .collect::<Vec<(String, Vec<u8>)>>()
        };

        if peer_targets.is_empty() {
            trace!(community_id, "broadcast: no peers in mesh");
            return BroadcastReport::default();
        }

        let sender = self.node.sender();
        let mut targets_with_routes = Vec::with_capacity(peer_targets.len());
        for (key, blob) in &peer_targets {
            match self.node.import_route(blob) {
                Ok(target) => targets_with_routes.push((key.clone(), target)),
                Err(e) => debug!(peer = %key, error = %e, "broadcast: route import failed"),
            }
        }

        sender
            .broadcast_raw_parallel(&targets_with_routes, data, 16)
            .await
    }
}
