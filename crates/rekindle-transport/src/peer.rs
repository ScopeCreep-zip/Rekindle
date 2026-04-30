//! Peer tracking, route caching, and circuit breaking.
//!
//! Manages known peers, their cached route blobs with staleness-aware
//! eviction, and per-peer circuit breakers that prevent flooding dead
//! routes with repeated timeout failures.

use std::collections::HashMap;
use std::time::Instant;

/// Opaque handle for a remote peer's route. Wraps a Veilid `RouteId`
/// without exposing it as a Veilid type through the public API.
#[derive(Debug, Clone)]
pub struct PeerTarget {
    pub(crate) route_id: veilid_core::RouteId,
}

/// Cached route blob with import timestamp for staleness detection.
#[derive(Debug, Clone)]
struct CachedRoute {
    blob: Vec<u8>,
    imported_at: Instant,
}

/// Per-peer failure tracking for circuit breaking.
#[derive(Debug)]
struct CircuitState {
    failure_count: u32,
    last_failure: Instant,
}

/// Registry of known peers with route caching and circuit breaking.
pub struct PeerRegistry {
    /// Route blob cache: peer_key → cached route.
    routes: HashMap<String, CachedRoute>,
    /// Circuit breaker state: peer_key → failure tracking.
    circuits: HashMap<String, CircuitState>,
    /// TTL for cached routes in seconds.
    route_ttl_secs: u64,
    /// Consecutive failures before the circuit trips.
    circuit_threshold: u32,
    /// Cooldown period in seconds after the circuit trips.
    circuit_cooldown_secs: u64,
}

impl PeerRegistry {
    pub fn new(route_ttl_secs: u64, circuit_threshold: u32, circuit_cooldown_secs: u64) -> Self {
        Self {
            routes: HashMap::new(),
            circuits: HashMap::new(),
            route_ttl_secs,
            circuit_threshold,
            circuit_cooldown_secs,
        }
    }

    /// Cache a peer's route blob.
    pub fn cache_route(&mut self, peer_key: &str, blob: Vec<u8>) {
        if blob.is_empty() {
            return;
        }
        self.routes.insert(
            peer_key.to_string(),
            CachedRoute {
                blob,
                imported_at: Instant::now(),
            },
        );
    }

    /// Get a cached route blob if it's not stale.
    pub fn get_route(&self, peer_key: &str) -> Option<&[u8]> {
        let cached = self.routes.get(peer_key)?;
        if cached.imported_at.elapsed().as_secs() > self.route_ttl_secs {
            return None;
        }
        Some(&cached.blob)
    }

    /// Remove a peer's cached route.
    pub fn invalidate_route(&mut self, peer_key: &str) {
        self.routes.remove(peer_key);
    }

    /// Evict all stale routes and return the keys of evicted peers.
    pub fn evict_stale_routes(&mut self) -> Vec<String> {
        let now = Instant::now();
        let stale_keys: Vec<String> = self
            .routes
            .iter()
            .filter(|(_, cached)| now.duration_since(cached.imported_at).as_secs() > self.route_ttl_secs)
            .map(|(key, _)| key.clone())
            .collect();

        for key in &stale_keys {
            self.routes.remove(key);
        }
        stale_keys
    }

    // ── Circuit breaker ──────────────────────────────────────────────

    /// Check if the circuit breaker is open (tripped) for a peer.
    pub fn is_circuit_open(&self, peer_key: &str) -> bool {
        match self.circuits.get(peer_key) {
            Some(state) => {
                state.failure_count >= self.circuit_threshold
                    && state.last_failure.elapsed().as_secs() < self.circuit_cooldown_secs
            }
            None => false,
        }
    }

    /// Record a failure against a peer's circuit breaker.
    pub fn record_failure(&mut self, peer_key: &str) {
        let entry = self
            .circuits
            .entry(peer_key.to_string())
            .or_insert(CircuitState {
                failure_count: 0,
                last_failure: Instant::now(),
            });
        entry.failure_count += 1;
        entry.last_failure = Instant::now();
    }

    /// Reset the circuit breaker for a peer on successful communication.
    pub fn reset_circuit(&mut self, peer_key: &str) {
        self.circuits.remove(peer_key);
    }
}

/// Public re-export for external use in circuit breaker checks.
pub type CircuitBreaker = PeerRegistry;
