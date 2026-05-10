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
    /// Blake3 hash of blob for cheap equality check on update.
    blob_hash: u64,
    imported_at: Instant,
    /// Cached imported RouteId. Invalidated when blob changes.
    imported_route_id: Option<veilid_core::RouteId>,
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

    /// Cache a peer's route blob. If the blob is unchanged (same hash),
    /// only refreshes the timestamp — preserves the cached RouteId.
    pub fn cache_route(&mut self, peer_key: &str, blob: Vec<u8>) {
        if blob.is_empty() {
            return;
        }
        let blob_hash = {
            let h = blake3::hash(&blob);
            u64::from_le_bytes(h.as_bytes()[..8].try_into().expect("blake3 is 32 bytes"))
        };
        // If blob unchanged, preserve existing RouteId and just refresh timestamp
        if let Some(existing) = self.routes.get_mut(peer_key) {
            if existing.blob_hash == blob_hash {
                existing.imported_at = Instant::now();
                return;
            }
        }
        self.routes.insert(
            peer_key.to_string(),
            CachedRoute {
                blob,
                blob_hash,
                imported_at: Instant::now(),
                imported_route_id: None,
            },
        );
    }

    /// Get a cached RouteId, importing only if needed.
    ///
    /// Returns `None` if no route cached or route is stale.
    /// On cache hit with valid RouteId: zero Veilid API calls.
    /// On cache hit without RouteId: calls `import_fn`, caches result.
    pub fn get_or_import(
        &mut self,
        peer_key: &str,
        import_fn: impl FnOnce(&[u8]) -> crate::error::Result<PeerTarget>,
    ) -> Option<crate::error::Result<PeerTarget>> {
        let cached = self.routes.get_mut(peer_key)?;
        if cached.imported_at.elapsed().as_secs() > self.route_ttl_secs {
            return None;
        }
        if let Some(ref route_id) = cached.imported_route_id {
            return Some(Ok(PeerTarget { route_id: route_id.clone() }));
        }
        let result = import_fn(&cached.blob);
        if let Ok(ref target) = result {
            cached.imported_route_id = Some(target.route_id.clone());
        }
        Some(result)
    }

    /// Get a cached route blob if it's not stale.
    pub fn get_route(&self, peer_key: &str) -> Option<&[u8]> {
        let cached = self.routes.get(peer_key)?;
        if cached.imported_at.elapsed().as_secs() > self.route_ttl_secs {
            return None;
        }
        Some(&cached.blob)
    }

    /// Remove a peer's cached route entirely.
    pub fn invalidate_route(&mut self, peer_key: &str) {
        self.routes.remove(peer_key);
    }

    /// Invalidate only the cached RouteId without removing the blob.
    /// Called on send failure — forces re-import on next `get_or_import()`.
    /// Wired by gossip broadcast failure handling in M1.
    #[allow(dead_code)]
    pub fn invalidate_route_id(&mut self, peer_key: &str) {
        if let Some(cached) = self.routes.get_mut(peer_key) {
            cached.imported_route_id = None;
        }
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

    /// Minimum age (seconds) before a circuit breaker entry can be evicted.
    /// Prevents an attacker from rapidly cycling pseudonyms to flush their
    /// own tripped circuit breaker.
    const MIN_CIRCUIT_ENTRY_AGE_SECS: u64 = 300;

    /// Record a failure against a peer's circuit breaker.
    ///
    /// If the circuits map exceeds `MAX_CIRCUIT_ENTRIES`, the oldest entry
    /// that is older than `MIN_CIRCUIT_ENTRY_AGE_SECS` is evicted. If all
    /// entries are younger than the minimum age, eviction is refused — the
    /// new peer won't get tracked (benign: it gets retried normally).
    pub fn record_failure(&mut self, peer_key: &str) {
        if self.circuits.len() >= Self::MAX_CIRCUIT_ENTRIES && !self.circuits.contains_key(peer_key) {
            if let Some(oldest_key) = self
                .circuits
                .iter()
                .filter(|(_, state)| state.last_failure.elapsed().as_secs() >= Self::MIN_CIRCUIT_ENTRY_AGE_SECS)
                .min_by_key(|(_, state)| state.last_failure)
                .map(|(k, _)| k.clone())
            {
                self.circuits.remove(&oldest_key);
            } else {
                // All entries younger than minimum age — refuse to evict.
                tracing::warn!(
                    capacity = Self::MAX_CIRCUIT_ENTRIES,
                    min_age_secs = Self::MIN_CIRCUIT_ENTRY_AGE_SECS,
                    "circuit breaker at capacity with all entries too young to evict"
                );
                return;
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

/// Re-exported from `rekindle_types::display` — the SSOT definitions.
pub use rekindle_types::display::{CircuitSummary, PeerSnapshot};

/// Public re-export for external use in circuit breaker checks.
pub type CircuitBreaker = PeerRegistry;
