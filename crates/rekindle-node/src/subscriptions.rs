//! Per-connection event subscription registry and fan-out delivery.
//!
//! Manages which connections receive which events. Serializes the event
//! payload once, then sends the serialized bytes to all matching
//! connections via their outbound channels.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_transport_ipc::v3::codec::datagram::publish as publish_codec;
use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::server::ConnectionHandle;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_kind::DatagramKind;
use rekindle_types::subscription_events::{SubscriptionEvent, SubscriptionFilter};

struct ConnSub {
    handle: ConnectionHandle,
    filters: Vec<SubscriptionFilter>,
}

/// Manages per-connection event subscriptions and fan-out delivery.
///
/// Thread-safe: `subscribe`, `unsubscribe`, `remove_connection` acquire
/// the write lock; `fan_out` acquires the read lock.
pub struct SubscriptionRegistry {
    conns: RwLock<HashMap<u64, ConnSub>>,
}

impl SubscriptionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            conns: RwLock::new(HashMap::new()),
        })
    }

    /// Register or extend a connection's subscription filters.
    pub fn subscribe(
        &self,
        conn_id: u64,
        handle: ConnectionHandle,
        filters: Vec<SubscriptionFilter>,
    ) {
        let mut conns = self.conns.write();
        conns
            .entry(conn_id)
            .and_modify(|s| s.filters.extend(filters.iter().cloned()))
            .or_insert_with(|| ConnSub { handle, filters });
    }

    /// Remove specific filters from a connection's subscription.
    pub fn unsubscribe(&self, conn_id: u64, filters: &[SubscriptionFilter]) {
        let mut conns = self.conns.write();
        if let Some(sub) = conns.get_mut(&conn_id) {
            sub.filters.retain(|f| !filters.contains(f));
            if sub.filters.is_empty() {
                conns.remove(&conn_id);
            }
        }
    }

    /// Remove all subscriptions for a connection (on disconnect).
    pub fn remove_connection(&self, conn_id: u64) {
        self.conns.write().remove(&conn_id);
    }

    /// Deliver an event to all connections with matching filters.
    ///
    /// Serialization is done ONCE. Channel sends are non-blocking
    /// (try_send). If a connection's outbound channel is full, the
    /// event is dropped for that connection (backpressure).
    pub fn fan_out(&self, event: &SubscriptionEvent) {
        let app_payload = match event.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "event serialize failed");
                return;
            }
        };

        let topic = event_topic_hash(event);

        let publish = publish_codec::encode(&publish_codec::DatagramPublishPayload {
            message_id: uuid::Uuid::now_v7(),
            topic_hash: topic,
            event_seq: 0,
            event_timestamp_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            sender_clearance: Clearance::Internal,
            application_payload: app_payload,
            conditions: vec![],
        });

        let conns = self.conns.read();
        for sub in conns.values() {
            if sub.filters.iter().any(|f| f.matches(event)) {
                let _ = sub.handle.outbound_tx.try_send(OutboundFrame::Datagram {
                    kind: DatagramKind::Publish,
                    payload: publish.clone(),
                });
            }
        }
    }

    /// Number of active subscribed connections.
    pub fn connection_count(&self) -> usize {
        self.conns.read().len()
    }
}

/// Compute a deterministic topic hash for an event.
fn event_topic_hash(event: &SubscriptionEvent) -> [u8; 32] {
    let cat = event.category();
    let community = event.community().unwrap_or("");
    *blake3::hash(format!("{cat:?}:{community}").as_bytes()).as_bytes()
}
