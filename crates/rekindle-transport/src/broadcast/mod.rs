//! Consolidated outbound module — the sole Veilid boundary for all outgoing data.
//!
//! Every way data leaves the node to Veilid lives here. No other module
//! in the workspace imports `veilid_core`. This is the strict outbound boundary.
//!
//! # Submodules
//!
//! ## Veilid lifecycle & infrastructure
//! - `node` — VeilidAPI lifecycle (startup, shutdown, attach, detach, RoutingContext)
//! - `send` — app_message / app_call outbound wrappers
//! - `peer_route` — route allocation, import, release (RouteManager)
//! - `peer_registry` — peer route caching and circuit breaking (PeerRegistry)
//! - `dht/` — all DHT record CRUD (create, open, close, get, set, watch, inspect)
//!
//! ## Application-level broadcast
//! - `dht_writes` — thin primitive wrappers over dht/ for TransportNode callers
//! - `gossip` — community mesh broadcast (all 52 GossipPayload + ControlPayload variants)
//! - `dm` — peer-to-peer DM sends (all 10 DmPayload variants)
//! - `rpc` — request-response RPC calls (governance, sync, leave)
//! - `voice` — voice packet send (single peer + mesh)
//! - `route` — route lifecycle convenience (allocate, refresh, publish)

// Veilid infrastructure (imports veilid_core)
pub mod node;
pub mod send;
pub mod peer_route;
pub mod peer_registry;
pub mod dht;

// Application-level broadcast (calls through infrastructure above)
pub mod dht_writes;
pub mod gossip;
pub mod dm;
pub mod rpc;
pub mod voice;
pub mod route;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::crypto::mek::MekCache;
use crate::gossip::GossipMesh;
use crate::session::Session;

use node::TransportNode;

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
pub struct BroadcastManager {
    pub(crate) node: Arc<TransportNode>,
    pub(crate) session: Arc<RwLock<Option<Session>>>,
    pub(crate) mek_cache: Arc<RwLock<MekCache>>,
    pub(crate) meshes: Arc<RwLock<HashMap<String, GossipMesh>>>,
    pub(crate) rate_limiter: RwLock<OutboundRateLimiter>,
}

impl BroadcastManager {
    pub fn new(
        node: Arc<TransportNode>,
        session: Arc<RwLock<Option<Session>>>,
        mek_cache: Arc<RwLock<MekCache>>,
    ) -> Self {
        Self {
            node, session, mek_cache,
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
    pub fn session(&self) -> &Arc<RwLock<Option<Session>>> { &self.session }
    pub fn mek_cache(&self) -> &Arc<RwLock<MekCache>> { &self.mek_cache }
    pub fn meshes(&self) -> &Arc<RwLock<HashMap<String, GossipMesh>>> { &self.meshes }
}
