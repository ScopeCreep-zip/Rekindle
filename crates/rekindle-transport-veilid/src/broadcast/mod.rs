//! Consolidated outbound module — the sole Veilid boundary for all outgoing data.
//!
//! # Submodules
//!
//! ## Veilid lifecycle & infrastructure
//! - `node` — VeilidAPI lifecycle, all subsystem construction, dispatch spawn
//! - `send` — app_message / app_call outbound wrappers (opaque bytes only)
//! - `peer_route` — route allocation, import, release (RouteManager)
//! - `peer_registry` — peer route caching and circuit breaking (PeerRegistry)
//! - `dht/` — all DHT record CRUD (create, open, close, get, set, watch, inspect)
//!
//! ## Broadcast helpers
//! - `dht_writes` — thin primitive wrappers over dht/ for TransportNode callers
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

use crate::config::TransportConfig;
use crate::gossip::GossipMesh;
use send::{BroadcastReport, Sender};

/// Rate limiter for outbound gossip, keyed by a string identifier.
#[derive(Debug, Default)]
pub struct OutboundRateLimiter {
    last_sent: HashMap<String, std::time::Instant>,
}

impl OutboundRateLimiter {
    pub fn check(&mut self, key: &str, min_interval: std::time::Duration) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.last_sent.get(key) {
            if now.duration_since(*last) < min_interval { return false; }
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
/// Takes raw primitives (VeilidAPI, Config) — NOT Arc<TransportNode>.
/// Constructed in TransportNode::start() before the node is returned.
pub struct BroadcastManager {
    api: veilid_core::VeilidAPI,
    config: Arc<TransportConfig>,
    pub(crate) meshes: Arc<RwLock<HashMap<String, GossipMesh>>>,
    pub(crate) rate_limiter: RwLock<OutboundRateLimiter>,
}

impl BroadcastManager {
    pub fn new(api: veilid_core::VeilidAPI, config: Arc<TransportConfig>) -> Self {
        Self {
            api,
            config,
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

    pub fn meshes(&self) -> &Arc<RwLock<HashMap<String, GossipMesh>>> { &self.meshes }

    /// Add or update a peer in a community's gossip mesh, then refresh peer set.
    pub fn upsert_mesh_peer(
        &self, community_id: &str, pseudonym: &str,
        route_blob: Vec<u8>, status: &str, now_secs: u64, my_pseudonym: &str,
    ) {
        let mut guard = self.meshes.write();
        let Some(mesh) = guard.get_mut(community_id) else {
            debug!(community_id, "upsert_mesh_peer: no mesh");
            return;
        };
        mesh.upsert_member(pseudonym.to_string(), crate::gossip::OnlineMember {
            route_blob, status: status.to_string(), last_seen: now_secs,
        });
        mesh.refresh_peer_set(my_pseudonym);
        tracing::info!(
            community_id, pseudonym = &pseudonym[..16.min(pseudonym.len())],
            online = mesh.online_members.len(), selected = mesh.peers.len(),
            "mesh peer upserted"
        );
    }

    /// Remove a peer from a community's gossip mesh.
    pub fn remove_mesh_peer(&self, community_id: &str, pseudonym: &str) {
        let mut guard = self.meshes.write();
        if let Some(mesh) = guard.get_mut(community_id) {
            mesh.remove_member(pseudonym);
            debug!(community_id, pseudonym = &pseudonym[..16.min(pseudonym.len())], "mesh peer removed");
        }
    }

    /// Fan out pre-signed, pre-framed bytes to all mesh peers for a community.
    pub async fn broadcast_to_mesh(&self, community_id: &str, data: &[u8]) -> BroadcastReport {
        let peer_targets = {
            let guard = self.meshes.read();
            let Some(mesh) = guard.get(community_id) else {
                debug!(community_id, "broadcast: no mesh");
                return BroadcastReport::default();
            };
            mesh.peers.iter()
                .map(|(k, m)| (k.clone(), m.route_blob.clone()))
                .collect::<Vec<(String, Vec<u8>)>>()
        };

        if peer_targets.is_empty() {
            trace!(community_id, "broadcast: no peers in mesh");
            return BroadcastReport::default();
        }

        let sender = Sender::new(self.api.clone(), Arc::clone(&self.config));
        let mut targets_with_routes = Vec::with_capacity(peer_targets.len());
        for (key, blob) in &peer_targets {
            match self.api.import_remote_private_route(blob.clone()) {
                Ok(route_id) => {
                    tracing::info!(
                        peer = &key[..16.min(key.len())],
                        route_id = %route_id,
                        blob_len = blob.len(),
                        "broadcast: route imported for peer"
                    );
                    targets_with_routes.push((key.clone(), peer_registry::PeerTarget { route_id }));
                }
                Err(e) => tracing::warn!(peer = &key[..16.min(key.len())], error = %e, blob_len = blob.len(), "broadcast: route import FAILED"),
            }
        }

        tracing::info!(
            community_id = &community_id[..20.min(community_id.len())],
            data_len = data.len(),
            targets = targets_with_routes.len(),
            type_id = data.first().copied().unwrap_or(0),
            "broadcast: sending to mesh peers"
        );
        let report = sender.broadcast_raw_parallel(&targets_with_routes, data, 16).await;
        tracing::info!(
            delivered = report.delivered,
            failed = report.failures.len(),
            "broadcast: parallel send complete"
        );
        report
    }
}
