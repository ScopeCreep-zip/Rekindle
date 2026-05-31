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
use crate::broadcast::peer_route::RouteManager;
use crate::crypto::mek::{Mek, MekCache, MekCacheEntrySnapshot};
use crate::shared::{AttachmentState, SharedState, TransportNotification, TransportSnapshot};

/// Mock transport node for testing without Veilid.
///
/// Provides the same observable interfaces as `TransportNode` —
/// `SharedState`, `PeerRegistry`, `RouteManager`, `MekCache` — but
/// backed by in-memory state that tests can control directly.
pub struct MockNode {
    shared: Arc<SharedState>,
    peer_registry: Arc<RwLock<PeerRegistry>>,
    route_manager: Arc<RwLock<RouteManager>>,
    mek_cache: Arc<RwLock<MekCache>>,
    /// Override for route_allocated in status_snapshot.
    /// RouteManager::set_route requires a veilid_core::RouteId which
    /// cannot be constructed without a Veilid runtime. This flag lets
    /// tests control the route_allocated field in status_snapshot().
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
            route_manager: Arc::new(RwLock::new(RouteManager::new())),
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

    /// Record a failure against a peer's circuit breaker.
    #[allow(dead_code)]
    pub fn fail_peer(&self, key: &str) {
        self.peer_registry.write().record_failure(key);
    }

    /// Trip a peer's circuit breaker by recording enough consecutive failures.
    pub fn trip_circuit(&self, key: &str) {
        let mut reg = self.peer_registry.write();
        for _ in 0..3 {
            reg.record_failure(key);
        }
    }

    /// Add a synthetic MEK to the cache.
    pub fn add_mek(&self, community_id: &str, channel_id: &str, generation: u64) {
        let mek = Mek::generate(generation);
        self.mek_cache.write().insert(community_id, channel_id, mek);
    }

    /// Mark the node as having an allocated route.
    ///
    /// `RouteManager::set_route` requires a `veilid_core::RouteId` which
    /// cannot be constructed without a Veilid runtime. This method sets
    /// an override flag that `status_snapshot()` reads instead.
    #[allow(dead_code)]
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

    /// Route manager.
    #[allow(dead_code)]
    pub fn routes(&self) -> &Arc<RwLock<RouteManager>> {
        &self.route_manager
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
    /// Uses `route_allocated_override` instead of `RouteManager::has_route()`
    /// because `RouteManager::set_route` requires a Veilid `RouteId`.
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
mod tests {
    use super::*;

    #[test]
    fn mock_node_default_is_detached() {
        let mock = MockNode::new();
        assert!(!mock.shared().is_attached());
        assert!(!mock.shared().public_internet_ready());
        assert_eq!(mock.shared().attachment_state(), AttachmentState::Detached);
    }

    #[test]
    fn mock_node_attached_constructor() {
        let mock = MockNode::attached();
        assert!(mock.shared().is_attached());
        assert!(mock.shared().public_internet_ready());
        assert_eq!(
            mock.shared().attachment_state(),
            AttachmentState::FullyAttached
        );
    }

    #[test]
    fn mock_node_add_peer() {
        let mock = MockNode::new();
        assert_eq!(mock.peers().read().route_count(), 0);

        mock.add_peer("peer1", vec![1, 2, 3]);
        assert_eq!(mock.peers().read().route_count(), 1);

        mock.add_peer("peer2", vec![4, 5, 6]);
        assert_eq!(mock.peers().read().route_count(), 2);
    }

    #[test]
    fn mock_node_circuit_breaker() {
        let mock = MockNode::new();
        mock.add_peer("peer1", vec![1, 2, 3]);

        assert!(!mock.peers().read().is_circuit_open("peer1"));

        mock.trip_circuit("peer1");
        assert!(mock.peers().read().is_circuit_open("peer1"));

        let summary = mock.circuit_summary();
        assert_eq!(summary.circuit_open, 1);
    }

    #[test]
    fn mock_node_mek_cache() {
        let mock = MockNode::new();
        mock.add_mek("comm1", "chan1", 1);
        mock.add_mek("comm1", "chan1", 2);
        mock.add_mek("comm1", "chan2", 1);

        let snapshot = mock.mek_snapshot("comm1");
        assert_eq!(snapshot.len(), 3);

        let cache = mock.mek_cache().read();
        assert_eq!(cache.current("comm1", "chan1").unwrap().generation(), 2);
        assert_eq!(cache.current("comm1", "chan2").unwrap().generation(), 1);
    }

    #[tokio::test]
    async fn mock_node_subscribe_receives_injected_events() {
        let mock = MockNode::new();
        let mut rx = mock.subscribe();

        mock.inject(&TransportNotification::DmReceived {
            sender_key: "abc123".into(),
            sender_name: "alice".into(),
            timestamp: 1234,
        });

        let event = rx.try_recv().expect("should receive event");
        match event {
            TransportNotification::DmReceived { sender_key, .. } => {
                assert_eq!(sender_key, "abc123");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn mock_node_status_snapshot() {
        let mock = MockNode::attached();
        mock.add_peer("p1", vec![1]);
        mock.add_peer("p2", vec![2]);

        let status = mock.status_snapshot();
        assert!(status.is_attached);
        assert!(status.public_internet_ready);
        assert_eq!(status.peer_count, 2);
        assert!(!status.route_allocated);

        // After setting the override, route_allocated should be true
        mock.set_route_allocated(true);
        let status2 = mock.status_snapshot();
        assert!(status2.route_allocated);
    }

    #[test]
    fn mock_node_peer_snapshot_sorted() {
        let mock = MockNode::new();
        mock.add_peer("zzz_peer", vec![1]);
        mock.add_peer("aaa_peer", vec![2]);

        let snapshot = mock.peer_snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].key, "aaa_peer");
        assert_eq!(snapshot[1].key, "zzz_peer");
    }

    #[test]
    fn mock_node_set_specific_attachment() {
        let mock = MockNode::new();
        mock.set_attachment_state(AttachmentState::AttachedWeak, true, false);

        assert!(mock.shared().is_attached());
        assert!(!mock.shared().public_internet_ready());
        assert_eq!(
            mock.shared().attachment_state(),
            AttachmentState::AttachedWeak
        );
    }

    #[test]
    fn mock_node_empty_route_blob_ignored() {
        let mock = MockNode::new();
        mock.add_peer("peer1", Vec::new()); // empty blob
        assert_eq!(mock.peers().read().route_count(), 0); // should not be cached
    }
}
