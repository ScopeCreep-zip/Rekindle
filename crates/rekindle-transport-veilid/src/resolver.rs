//! RouteResolver — maps peer identifiers to Veilid route blobs with caching.
//!
//! The resolver is the single source of truth for "how do I reach peer X?"
//! It maintains two data structures:
//!
//! 1. `peer_to_profile`: maps peer_key (identity hex or pseudonym hex) →
//!    profile_dht_key (VLD0:...). Populated by chat layer when it learns
//!    the association (friend request, member registry, inbox scan).
//!
//! 2. `PeerRegistry`: maps peer_key → cached route_blob with TTL and
//!    circuit breaker. RouteResolver writes to it after successful DHT reads.
//!
//! Takes raw primitives (PeerRegistry, VeilidAPI, TransportConfig) — NOT
//! Arc<TransportNode>. No circular dependency. Constructed in
//! TransportNode::start() before the node itself is returned.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{debug, info, warn};

use crate::broadcast::peer_registry::{PeerRegistry, PeerTarget};
use crate::config::TransportConfig;

/// Profile DHT subkey containing the route blob.
const PROFILE_SUBKEY_ROUTE_BLOB: u32 = 6;

/// Route resolution result.
pub enum ResolveResult {
    /// Route found — ready to send via this target.
    Found(PeerTarget),
    /// No profile_dht_key registered for this peer.
    UnknownPeer,
    /// Profile known but route blob empty or read failed.
    NoRoute,
    /// Circuit breaker tripped — too many recent failures.
    CircuitOpen,
}

pub struct RouteResolver {
    peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
    api: veilid_core::VeilidAPI,
    config: Arc<TransportConfig>,
    peer_to_profile: RwLock<HashMap<String, String>>,
}

impl RouteResolver {
    pub fn new(
        peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
        api: veilid_core::VeilidAPI,
        config: Arc<TransportConfig>,
    ) -> Self {
        Self {
            peer_registry,
            api,
            config,
            peer_to_profile: RwLock::new(HashMap::new()),
        }
    }

    /// Register the mapping from a peer identifier to their profile DHT key.
    pub fn register_peer(&self, peer_key: &str, profile_dht_key: &str) {
        if peer_key.is_empty() || profile_dht_key.is_empty() { return; }
        self.peer_to_profile.write().insert(peer_key.to_string(), profile_dht_key.to_string());
        debug!(
            peer = &peer_key[..16.min(peer_key.len())],
            profile = &profile_dht_key[..20.min(profile_dht_key.len())],
            "peer→profile mapping registered"
        );
    }

    /// Register profile_dht_key as its own peer_key for callers that only have the profile key.
    pub fn register_profile_self(&self, profile_dht_key: &str) {
        if profile_dht_key.is_empty() { return; }
        self.peer_to_profile.write().insert(profile_dht_key.to_string(), profile_dht_key.to_string());
    }

    /// Look up the profile_dht_key for a peer.
    pub fn profile_key_for(&self, peer_key: &str) -> Option<String> {
        self.peer_to_profile.read().get(peer_key).cloned()
    }

    /// Resolve a peer_key to a sendable PeerTarget.
    ///
    /// 1. Check PeerRegistry cache (O(1), zero DHT reads)
    /// 2. If miss: look up profile_dht_key → DHT read subkey 6 → import → cache
    pub async fn resolve(&self, peer_key: &str) -> ResolveResult {
        // Step 1: Circuit breaker check
        {
            let registry = self.peer_registry.read();
            if registry.is_circuit_open(peer_key) {
                return ResolveResult::CircuitOpen;
            }
        }

        // Step 2: Cache hit (hot path)
        {
            let mut registry = self.peer_registry.write();
            if let Some(result) = registry.get_or_import(peer_key, |blob| {
                self.import_route(blob)
            }) {
                match result {
                    Ok(target) => return ResolveResult::Found(target),
                    Err(e) => {
                        debug!(peer = &peer_key[..16.min(peer_key.len())], error = %e, "cached route import failed — DHT refresh");
                        registry.invalidate_route(peer_key);
                    }
                }
            }
        }

        // Step 3: Look up profile_dht_key
        let profile_key = match self.peer_to_profile.read().get(peer_key) {
            Some(pk) => pk.clone(),
            None => {
                debug!(peer = &peer_key[..16.min(peer_key.len())], "no profile_dht_key — cannot resolve");
                return ResolveResult::UnknownPeer;
            }
        };

        // Step 4: DHT read route blob
        let route_blob = match self.read_route_blob(&profile_key).await {
            Some(blob) => blob,
            None => return ResolveResult::NoRoute,
        };

        // Step 5: Cache and import
        {
            self.peer_registry.write().cache_route(peer_key, route_blob.clone());
        }

        match self.import_route(&route_blob) {
            Ok(target) => {
                let mut registry = self.peer_registry.write();
                let _ = registry.get_or_import(peer_key, |_| Ok(target.clone()));
                info!(peer = &peer_key[..16.min(peer_key.len())], "route resolved via DHT");
                ResolveResult::Found(target)
            }
            Err(e) => {
                warn!(peer = &peer_key[..16.min(peer_key.len())], error = %e, "route import failed after DHT read");
                ResolveResult::NoRoute
            }
        }
    }

    /// Invalidate a peer's cached route.
    pub fn invalidate(&self, peer_key: &str) {
        self.peer_registry.write().invalidate_route(peer_key);
        debug!(peer = &peer_key[..16.min(peer_key.len())], "route invalidated");
    }

    /// Record a send failure for circuit breaker tracking.
    pub fn record_failure(&self, peer_key: &str) {
        self.peer_registry.write().record_failure(peer_key);
    }

    /// Reset circuit breaker on successful communication.
    pub fn record_success(&self, peer_key: &str) {
        self.peer_registry.write().reset_circuit(peer_key);
    }

    pub fn registered_peer_count(&self) -> usize { self.peer_to_profile.read().len() }
    pub fn cached_route_count(&self) -> usize { self.peer_registry.read().route_count() }

    // ── Internal — uses VeilidAPI directly, no Arc<TransportNode> ───

    fn import_route(&self, route_blob: &[u8]) -> std::result::Result<PeerTarget, crate::error::TransportError> {
        let route_id = self.api.import_remote_private_route(route_blob.to_vec())
            .map_err(|e| crate::error::TransportError::RouteImportFailed {
                peer: String::new(),
                reason: format!("{e}"),
            })?;
        Ok(PeerTarget { route_id })
    }

    async fn read_route_blob(&self, profile_dht_key: &str) -> Option<Vec<u8>> {
        let rc = match crate::broadcast::node::build_routing_context(&self.api, &self.config.safety.dht) {
            Ok(rc) => rc,
            Err(e) => {
                debug!(error = %e, "routing context build failed for route blob read");
                return None;
            }
        };

        let record_key: veilid_core::RecordKey = match profile_dht_key.parse() {
            Ok(k) => k,
            Err(e) => {
                debug!(profile = &profile_dht_key[..20.min(profile_dht_key.len())], error = %e, "invalid profile DHT key");
                return None;
            }
        };

        // Open read-only
        if let Err(e) = rc.open_dht_record(record_key.clone(), None).await {
            debug!(profile = &profile_dht_key[..20.min(profile_dht_key.len())], error = %e, "profile open failed");
            return None;
        }

        match rc.get_dht_value(record_key, PROFILE_SUBKEY_ROUTE_BLOB, true).await {
            Ok(Some(value_data)) => {
                let blob = value_data.data().to_vec();
                if blob.is_empty() {
                    debug!(profile = &profile_dht_key[..20.min(profile_dht_key.len())], "route blob empty");
                    None
                } else {
                    debug!(profile = &profile_dht_key[..20.min(profile_dht_key.len())], blob_len = blob.len(), "route blob read");
                    Some(blob)
                }
            }
            Ok(None) => {
                debug!(profile = &profile_dht_key[..20.min(profile_dht_key.len())], "route blob subkey not written");
                None
            }
            Err(e) => {
                warn!(profile = &profile_dht_key[..20.min(profile_dht_key.len())], error = %e, "route blob DHT read failed");
                None
            }
        }
    }
}
