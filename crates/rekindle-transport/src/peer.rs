//! Peer tracking, route caching, and circuit breaking.
//!
//! Manages known peers, their cached route blobs with staleness-aware
//! eviction, and per-peer circuit breakers that prevent flooding dead
//! routes with repeated timeout failures.

use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;

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

    /// Maximum number of circuit breaker entries to prevent memory exhaustion
    /// from an attacker sending from many pseudonyms.
    const MAX_CIRCUIT_ENTRIES: usize = 4096;

    /// Record a failure against a peer's circuit breaker.
    ///
    /// If the circuits map exceeds `MAX_CIRCUIT_ENTRIES`, the oldest entry
    /// (by `last_failure` timestamp) is evicted before inserting. This bounds
    /// memory growth from attackers using many pseudonyms.
    pub fn record_failure(&mut self, peer_key: &str) {
        // Evict oldest if at capacity and this is a new key
        if self.circuits.len() >= Self::MAX_CIRCUIT_ENTRIES && !self.circuits.contains_key(peer_key) {
            if let Some(oldest_key) = self
                .circuits
                .iter()
                .min_by_key(|(_, state)| state.last_failure)
                .map(|(k, _)| k.clone())
            {
                self.circuits.remove(&oldest_key);
            }
        }

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

    // ── Introspection (for CLI/TUI display) ─────────────────────────

    /// Number of known peers with valid (non-stale) cached routes.
    pub fn route_count(&self) -> usize {
        self.routes
            .iter()
            .filter(|(_, cached)| cached.imported_at.elapsed().as_secs() <= self.route_ttl_secs)
            .count()
    }

    /// Total number of cached routes, including stale ones.
    pub fn total_cached(&self) -> usize {
        self.routes.len()
    }

    /// Number of peers with tripped circuit breakers.
    pub fn circuit_open_count(&self) -> usize {
        self.circuits
            .keys()
            .filter(|key| self.is_circuit_open(key))
            .count()
    }

    /// Summary of peer health states for dashboard display.
    pub fn circuit_summary(&self) -> CircuitSummary {
        let total = self.routes.len();
        let healthy = self.route_count();
        let circuit_open = self.circuit_open_count();
        let degraded = total.saturating_sub(healthy).saturating_sub(circuit_open);
        CircuitSummary {
            total,
            healthy,
            degraded,
            circuit_open,
        }
    }

    /// Point-in-time snapshot of all known peers for display.
    ///
    /// Returns display-ready data — no locks needed by the caller.
    /// Sorted by key for stable display ordering.
    pub fn snapshot(&self) -> Vec<PeerSnapshot> {
        let mut peers: Vec<PeerSnapshot> = self
            .routes
            .iter()
            .map(|(key, cached)| {
                let age_secs = cached.imported_at.elapsed().as_secs();
                let is_stale = age_secs > self.route_ttl_secs;
                let circuit = self.circuits.get(key);
                let circuit_open = self.is_circuit_open(key);
                let failure_count = circuit.map_or(0, |c| c.failure_count);

                let key_short = if key.len() > 12 {
                    format!("{}…{}", &key[..8], &key[key.len() - 4..])
                } else {
                    key.clone()
                };

                PeerSnapshot {
                    key: key.clone(),
                    key_short,
                    has_route: !is_stale,
                    route_age_secs: age_secs,
                    circuit_open,
                    failure_count,
                }
            })
            .collect();

        peers.sort_by(|a, b| a.key.cmp(&b.key));
        peers
    }
}

/// Summary of peer health states.
#[derive(Debug, Clone, Serialize)]
pub struct CircuitSummary {
    /// Total known peers (including stale routes).
    pub total: usize,
    /// Peers with valid routes and closed circuit breakers.
    pub healthy: usize,
    /// Peers with stale routes but closed circuit breakers.
    pub degraded: usize,
    /// Peers with tripped circuit breakers.
    pub circuit_open: usize,
}

/// Point-in-time snapshot of a single peer for display.
#[derive(Debug, Clone, Serialize)]
pub struct PeerSnapshot {
    /// Full peer public key (hex).
    pub key: String,
    /// Abbreviated key for compact display.
    pub key_short: String,
    /// Whether the cached route is still within TTL.
    pub has_route: bool,
    /// Seconds since the route was cached.
    pub route_age_secs: u64,
    /// Whether the circuit breaker is currently tripped.
    pub circuit_open: bool,
    /// Consecutive failure count (may be > 0 even with closed breaker if cooldown elapsed).
    pub failure_count: u32,
}

/// Public re-export for external use in circuit breaker checks.
pub type CircuitBreaker = PeerRegistry;
