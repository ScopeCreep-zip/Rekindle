//! Peer tracking, route caching, and circuit breaking.
//!
//! Manages known peers, their cached route blobs with staleness-aware
//! eviction, and per-peer circuit breakers that prevent flooding dead
//! routes with repeated timeout failures.

use std::collections::HashMap;
use tokio::time::Instant;

use tokio::sync::broadcast;

/// How many route changes a slow subscriber may fall behind by.
const UPDATES_CAPACITY: usize = 64;

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
    /// The peer whose cached route changed, for whoever follows routes
    /// (a live call's roster heal).
    updates: broadcast::Sender<String>,
}

impl PeerRegistry {
    pub fn new(route_ttl_secs: u64, circuit_threshold: u32, circuit_cooldown_secs: u64) -> Self {
        Self {
            routes: HashMap::new(),
            circuits: HashMap::new(),
            route_ttl_secs,
            circuit_threshold,
            circuit_cooldown_secs,
            updates: broadcast::channel(UPDATES_CAPACITY).0,
        }
    }

    /// Cache a peer's route blob; a new blob is announced on
    /// [`updates`](Self::updates).
    pub fn cache_route(&mut self, peer_key: &str, blob: Vec<u8>) {
        if blob.is_empty() {
            return;
        }
        let changed = self
            .routes
            .get(peer_key)
            .is_none_or(|cached| cached.blob != blob);
        self.routes.insert(
            peer_key.to_string(),
            CachedRoute {
                blob,
                imported_at: Instant::now(),
            },
        );
        if changed {
            // No subscriber is fine: nobody follows routes right now.
            let _ = self.updates.send(peer_key.to_string());
        }
    }

    /// The peers whose cached route changes from now on.
    pub fn updates(&self) -> broadcast::Receiver<String> {
        self.updates.subscribe()
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

    /// Forget the peers whose cached blob is one of `dead` (routes Veilid
    /// reported dead), so their next send fetches a fresh route. Returns
    /// those peers' keys.
    pub fn invalidate_blobs(&mut self, dead: &[Vec<u8>]) -> Vec<String> {
        let gone: Vec<String> = self
            .routes
            .iter()
            .filter(|(_, cached)| dead.contains(&cached.blob))
            .map(|(key, _)| key.clone())
            .collect();
        for key in &gone {
            self.routes.remove(key);
        }
        gone
    }

    /// Evict all stale routes and return the keys of evicted peers.
    pub fn evict_stale_routes(&mut self) -> Vec<String> {
        let now = Instant::now();
        let stale_keys: Vec<String> = self
            .routes
            .iter()
            .filter(|(_, cached)| {
                now.duration_since(cached.imported_at).as_secs() > self.route_ttl_secs
            })
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
        if self.circuits.len() >= Self::MAX_CIRCUIT_ENTRIES && !self.circuits.contains_key(peer_key)
        {
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

                let key_short = rekindle_utils::text::abbreviate(key, 8, 4);

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

/// Strand Relay sender state beside the route cache (plan E2.1): the
/// status-probe cooldown per target (§13.5). The relay routes' circuit
/// breakers (Wave 7 P7.3, `rekindle_route::relay`) join it at E5.5 with
/// the relay send.
#[derive(Default)]
pub struct RelayState {
    probe_cooldown: parking_lot::Mutex<HashMap<String, u64>>,
    /// The probes awaiting an answer: `(friend asked, target)` and when
    /// (plan audit G8).
    pending_probes: parking_lot::Mutex<HashMap<(String, String), u64>>,
}

impl RelayState {
    /// Whether a probe for `target` may go now (none in the last
    /// `cooldown_secs`); taking the slot when it may. Entries older than
    /// four cooldowns are dropped so the map stays small.
    pub fn take_probe_slot(&self, target: &str, now_secs: u64, cooldown_secs: u64) -> bool {
        let mut cooldown = self.probe_cooldown.lock();
        if let Some(&last) = cooldown.get(target) {
            if now_secs.saturating_sub(last) < cooldown_secs {
                return false;
            }
        }
        cooldown.insert(target.to_string(), now_secs);
        let horizon = cooldown_secs.saturating_mul(4);
        cooldown.retain(|_, &mut t| now_secs.saturating_sub(t) <= horizon);
        true
    }

    /// Remember that `friend` was asked about `target` at `now_secs`.
    /// Probes older than `ttl_secs` are dropped so the map stays small.
    pub fn record_probe(&self, friend: &str, target: &str, now_secs: u64, ttl_secs: u64) {
        let mut pending = self.pending_probes.lock();
        pending.retain(|_, &mut t| now_secs.saturating_sub(t) <= ttl_secs);
        pending.insert((friend.to_string(), target.to_string()), now_secs);
    }

    /// Whether `friend` answering about `target` at `now_secs` answers a
    /// probe of ours sent within `ttl_secs`; the probe is used up, so one
    /// probe takes one answer.
    pub fn take_probe_answer(
        &self,
        friend: &str,
        target: &str,
        now_secs: u64,
        ttl_secs: u64,
    ) -> bool {
        self.pending_probes
            .lock()
            .remove(&(friend.to_string(), target.to_string()))
            .is_some_and(|asked| now_secs.saturating_sub(asked) <= ttl_secs)
    }

    /// Forget the probe cooldowns and pending probes (lock): the next
    /// identity's probes are its own.
    pub fn clear_probes(&self) {
        self.probe_cooldown.lock().clear();
        self.pending_probes.lock().clear();
    }
}

/// Re-exported from `rekindle_types::display` — the SSOT definitions.
pub use rekindle_types::display::{CircuitSummary, PeerSnapshot};

/// Public re-export for external use in circuit breaker checks.
pub type CircuitBreaker = PeerRegistry;
