//! Mock transport node for testing without Veilid.
//!
//! [`MockNode`] creates a synthetic environment with controllable state
//! — no Veilid runtime, no network, no DHT. Used by test suites to
//! verify transport behavior without a live node.
//!
//! MockNode provides transport-level state only: attachment, peers, routes.
//! Application state (MEKs, sessions, unread counts) is in rekindle-chat.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::broadcast::peer_registry::{PeerRegistry, PeerSnapshot, CircuitSummary};
use crate::broadcast::peer_route::RouteManager;
use crate::shared::{AttachmentState, TransportSnapshot, SharedState, TransportNotification};

/// Mock transport node for testing without Veilid.
///
/// Provides the same observable interfaces as `TransportNode` —
/// `SharedState`, `PeerRegistry`, `RouteManager` — but backed by
/// in-memory state that tests can control directly.
///
/// Does NOT hold MekCache, SessionCache, or any application state.
/// Those are rekindle-chat concerns, tested independently.
pub struct MockNode {
    shared: Arc<SharedState>,
    peer_registry: Arc<RwLock<PeerRegistry>>,
    route_manager: Arc<RwLock<RouteManager>>,
    route_allocated_override: std::sync::atomic::AtomicBool,
}

impl MockNode {
    pub fn new() -> Self {
        let shared = SharedState::new();
        Self {
            shared,
            peer_registry: Arc::new(RwLock::new(PeerRegistry::new(90, 3, 45))),
            route_manager: Arc::new(RwLock::new(RouteManager::new())),
            route_allocated_override: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn attached() -> Self {
        let node = Self::new();
        node.set_attached(true);
        node
    }

    pub fn set_attached(&self, attached: bool) {
        let state = if attached {
            AttachmentState::FullyAttached
        } else {
            AttachmentState::Detached
        };
        self.shared.set_attachment(state, attached, attached);
    }

    pub fn set_attachment_state(
        &self,
        state: AttachmentState,
        attached: bool,
        public_internet_ready: bool,
    ) {
        self.shared.set_attachment(state, attached, public_internet_ready);
    }

    pub fn add_peer(&self, key: &str, route_blob: Vec<u8>) {
        self.peer_registry.write().cache_route(key, route_blob);
    }

    #[allow(dead_code)]
    pub fn fail_peer(&self, key: &str) {
        self.peer_registry.write().record_failure(key);
    }

    pub fn trip_circuit(&self, key: &str) {
        let mut reg = self.peer_registry.write();
        for _ in 0..3 {
            reg.record_failure(key);
        }
    }

    #[allow(dead_code)]
    pub fn set_route_allocated(&self, allocated: bool) {
        self.route_allocated_override
            .store(allocated, std::sync::atomic::Ordering::Release);
    }

    pub fn inject(&self, event: &TransportNotification) {
        self.shared.notify(event);
    }

    pub fn shared(&self) -> &Arc<SharedState> {
        &self.shared
    }

    pub fn peers(&self) -> &Arc<RwLock<PeerRegistry>> {
        &self.peer_registry
    }

    #[allow(dead_code)]
    pub fn routes(&self) -> &Arc<RwLock<RouteManager>> {
        &self.route_manager
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TransportNotification> {
        self.shared.subscribe()
    }

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
            route_age_secs: None,
        }
    }

    pub fn peer_snapshot(&self) -> Vec<PeerSnapshot> {
        self.peer_registry.read().snapshot()
    }

    pub fn circuit_summary(&self) -> CircuitSummary {
        self.peer_registry.read().circuit_summary()
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
        assert_eq!(mock.shared().attachment_state(), AttachmentState::FullyAttached);
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
        assert_eq!(mock.shared().attachment_state(), AttachmentState::AttachedWeak);
    }

    #[test]
    fn mock_node_empty_route_blob_ignored() {
        let mock = MockNode::new();
        mock.add_peer("peer1", Vec::new());
        assert_eq!(mock.peers().read().route_count(), 0);
    }
}
