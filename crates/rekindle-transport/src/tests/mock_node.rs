//! Mock transport node for CLI/TUI testing.
//!
//! [`MockNode`] creates a synthetic environment with controllable state
//! — no Veilid runtime, no network, no DHT. Used by M1/M2/M3 test suites
//! to verify CLI command behavior and TUI rendering without a live node.
//!
//! # Usage
//!
//! ```rust,ignore
//! let mock = MockNode::new();
//! mock.set_attached(true);
//! mock.add_peer("abc123...", vec![1, 2, 3]);
//! mock.add_mek("community-1", "channel-1", 1);
//!
//! // Inject events that subscribers receive
//! mock.inject(TransportNotification::DmReceived { ... });
//!
//! // Read state
//! assert!(mock.shared().is_attached());
//! assert_eq!(mock.peers().read().route_count(), 1);
//! ```

use std::sync::Arc;

use parking_lot::RwLock;

use crate::broadcast::peer_registry::{CircuitSummary, PeerRegistry, PeerSnapshot};
use crate::crypto::mek::{Mek, MekCache, MekCacheEntrySnapshot};
use crate::shared::{AttachmentState, SharedState, TransportNotification, TransportSnapshot};

/// Mock transport node for testing without Veilid.
///
/// Provides the same observable interfaces as `TransportNode` —
/// `SharedState`, `PeerRegistry`, `MekCache` — but backed by in-memory
/// state that tests can control directly.
///
/// No route owner is held: `OwnRoutes` allocates through a running
/// Veilid, so route state is modeled via `route_allocated_override`.
pub struct MockNode {
    shared: Arc<SharedState>,
    peer_registry: Arc<RwLock<PeerRegistry>>,
    mek_cache: Arc<RwLock<MekCache>>,
    /// Override for route_allocated in status_snapshot: lets tests control
    /// the field without a Veilid runtime to allocate a route.
    route_allocated_override: std::sync::atomic::AtomicBool,
}

impl MockNode {
    /// Create a new mock node in the default detached state.
    pub fn new() -> Self {
        let shared = SharedState::new();
        Self {
            shared,
            peer_registry: Arc::new(RwLock::new(PeerRegistry::new(
                90, // route_ttl_secs
                3,  // circuit_breaker_threshold
                45, // circuit_breaker_cooldown_secs
            ))),
            mek_cache: Arc::new(RwLock::new(MekCache::new())),
            route_allocated_override: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Create a mock node pre-configured as fully attached and ready.
    pub fn attached() -> Self {
        let node = Self::new();
        node.set_attached(true);
        node
    }

    // ── State manipulation ──────────────────────────────────────────

    /// Set the attachment state. Also updates `is_attached` and
    /// `public_internet_ready` to match.
    pub fn set_attached(&self, attached: bool) {
        let state = if attached {
            AttachmentState::FullyAttached
        } else {
            AttachmentState::Detached
        };
        self.shared.set_attachment(state, attached, attached);
    }

    /// Set a specific attachment state with fine-grained control.
    pub fn set_attachment_state(
        &self,
        state: AttachmentState,
        attached: bool,
        public_internet_ready: bool,
    ) {
        self.shared
            .set_attachment(state, attached, public_internet_ready);
    }

    /// Add a peer with a synthetic route blob.
    pub fn add_peer(&self, key: &str, route_blob: Vec<u8>) {
        self.peer_registry.write().cache_route(key, route_blob);
    }

    /// Trip a peer's circuit breaker by recording enough consecutive failures.
    pub fn trip_circuit(&self, key: &str) {
        let mut reg = self.peer_registry.write();
        for _ in 0..3 {
            reg.record_failure(key);
        }
    }

    /// Add a synthetic MEK to the cache.
    pub fn add_mek(
        &self,
        community_id: &str,
        scope: rekindle_types::channel_keys::KeyScope,
        generation: u64,
    ) {
        let mek = Mek::generate(generation);
        self.mek_cache.write().insert(community_id, scope, mek);
    }

    /// Mark the node as having an allocated route.
    ///
    /// Allocating a real route needs a Veilid runtime; this sets the
    /// override flag `status_snapshot()` reads instead.
    pub fn set_route_allocated(&self, allocated: bool) {
        self.route_allocated_override
            .store(allocated, std::sync::atomic::Ordering::Release);
    }

    /// Inject a synthetic transport notification.
    ///
    /// All current subscribers will receive this event.
    pub fn inject(&self, event: &TransportNotification) {
        self.shared.notify(event);
    }

    // ── Accessors (match TransportNode's public API) ────────────────

    /// Observable shared state.
    pub fn shared(&self) -> &Arc<SharedState> {
        &self.shared
    }

    /// Peer registry.
    pub fn peers(&self) -> &Arc<RwLock<PeerRegistry>> {
        &self.peer_registry
    }

    /// MEK cache.
    pub fn mek_cache(&self) -> &Arc<RwLock<MekCache>> {
        &self.mek_cache
    }

    /// Subscribe to transport notifications.
    pub fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<TransportNotification> {
        self.shared.subscribe()
    }

    /// Point-in-time status snapshot (matches `TransportNode::status_snapshot`).
    ///
    /// Uses `route_allocated_override` in place of the route owner's state.
    pub fn status_snapshot(&self) -> TransportSnapshot {
        let peer_reg = self.peer_registry.read();
        let route_allocated = self
            .route_allocated_override
            .load(std::sync::atomic::Ordering::Acquire);
        TransportSnapshot {
            attachment: self.shared.attachment_state().to_string(),
            is_attached: self.shared.is_attached(),
            public_internet_ready: self.shared.public_internet_ready(),
            uptime_secs: self.shared.uptime().as_secs(),
            peer_count: peer_reg.route_count(),
            route_allocated,
            route_age_secs: None, // No real route to measure age of
            general_route: if route_allocated {
                rekindle_types::subscription_events::RouteAvailability::Available
            } else {
                rekindle_types::subscription_events::RouteAvailability::Idle
            },
            route_paused: None, // No budget on the mock
            own_route_down: rekindle_types::subscription_events::OwnRouteDownSet::default(),
        }
    }

    /// Peer registry snapshot.
    pub fn peer_snapshot(&self) -> Vec<PeerSnapshot> {
        self.peer_registry.read().snapshot()
    }

    /// Circuit breaker summary.
    pub fn circuit_summary(&self) -> CircuitSummary {
        self.peer_registry.read().circuit_summary()
    }

    /// MEK cache snapshot for a community.
    pub fn mek_snapshot(&self, community_id: &str) -> Vec<MekCacheEntrySnapshot> {
        self.mek_cache.read().snapshot(community_id)
    }
}

impl Default for MockNode {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "mock_node_tests.rs"]
mod tests;
