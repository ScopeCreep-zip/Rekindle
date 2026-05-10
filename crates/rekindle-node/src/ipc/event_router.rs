//! Inverted-index event router for O(1) per-event delivery at 100K+ agent scale.
//!
//! On subscribe: conn_id inserted into category/community index sets.
//! On event arrival: 3 hash lookups produce exact recipient set. No per-connection
//! filter scanning. Serialization happens once per event, not once per recipient.
//!
//! The router lives on the IPC bus server's `ServerState`, not on `DaemonContext`.
//! The daemon sends events INTO the bus; the server routes them to subscribers.


use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use rekindle_types::subscription_events::{
    EventCategory, SubscriptionEvent, SubscriptionFilter,
    MAX_FILTERS_PER_CONNECTION,
};

use crate::ipc::framing::encode_frame;
use crate::ipc::message::{Message, SecurityLevel, Timestamp, WIRE_VERSION};
use crate::ipc::protocol::BusPayload;

/// Maximum subscribable connections. Bounds memory at scale.
const MAX_SUBSCRIBED_CONNECTIONS: usize = 100_000;

/// All 12 event categories for wildcard community-scoped expansion.
const ALL_CATEGORIES: [EventCategory; 12] = [
    EventCategory::ChannelMessage,
    EventCategory::Typing,
    EventCategory::Presence,
    EventCategory::Membership,
    EventCategory::Friend,
    EventCategory::Crypto,
    EventCategory::Voice,
    EventCategory::Governance,
    EventCategory::Social,
    EventCategory::Network,
    EventCategory::System,
    EventCategory::UnreadChanged,
];

/// Inverted-index event router.
///
/// Write-locked on subscribe/unsubscribe (rare, O(filters)).
/// Read-locked on deliver (hot path, O(1) lookups + O(recipients) sends).
pub struct EventRouter {
    /// conn_ids subscribed to ALL events.
    wildcard: HashSet<u64>,
    /// EventCategory → conn_ids subscribed to that category across all communities.
    by_category: HashMap<EventCategory, HashSet<u64>>,
    /// (EventCategory, community_gov_key) → conn_ids for that category + community.
    by_community: HashMap<(EventCategory, String), HashSet<u64>>,
    /// conn_id → outbound channel for serialized IPC frames.
    channels: HashMap<u64, mpsc::Sender<Vec<u8>>>,
    /// conn_id → original filters (for unsubscribe matching and diagnostics).
    original_filters: HashMap<u64, Vec<SubscriptionFilter>>,
    /// Monotonic epoch for timestamp generation in delivered frames.
    epoch: Instant,
}

impl EventRouter {
    pub fn new() -> Self {
        Self {
            wildcard: HashSet::new(),
            by_category: HashMap::new(),
            by_community: HashMap::new(),
            channels: HashMap::new(),
            original_filters: HashMap::new(),
            epoch: Instant::now(),
        }
    }

    /// Register a connection's filters and delivery channel.
    pub fn subscribe(
        &mut self,
        conn_id: u64,
        filters: &[SubscriptionFilter],
        tx: mpsc::Sender<Vec<u8>>,
    ) -> Result<usize, &'static str> {
        if self.channels.len() >= MAX_SUBSCRIBED_CONNECTIONS && !self.channels.contains_key(&conn_id) {
            warn!(conn_id, max = MAX_SUBSCRIBED_CONNECTIONS, "event_router: max connections exceeded");
            return Err("maximum subscribed connections exceeded");
        }

        let existing_count = self.original_filters.get(&conn_id).map_or(0, Vec::len);
        let remaining = MAX_FILTERS_PER_CONNECTION.saturating_sub(existing_count);
        if filters.len() > remaining {
            return Err("maximum filters per connection exceeded");
        }

        self.channels.insert(conn_id, tx);

        for filter in filters {
            self.index_filter(conn_id, filter);
        }

        self.original_filters.entry(conn_id).or_default().extend_from_slice(filters);
        let total = existing_count + filters.len();

        info!(conn_id, new_filters = filters.len(), total_filters = total, "event_router: subscribed");
        Ok(total)
    }

    /// Remove matching filters for a connection.
    pub fn unsubscribe(&mut self, conn_id: u64, filters: &[SubscriptionFilter]) -> usize {
        let remaining = if let Some(existing) = self.original_filters.get_mut(&conn_id) {
            for remove in filters {
                existing.retain(|f| {
                    f.categories != remove.categories || f.community_scope != remove.community_scope
                });
            }
            existing.clone()
        } else {
            return 0;
        };

        self.deindex_connection(conn_id);
        for filter in &remaining {
            self.index_filter(conn_id, filter);
        }

        remaining.len()
    }

    /// Remove a connection entirely.
    pub fn remove_connection(&mut self, conn_id: u64) {
        self.deindex_connection(conn_id);
        self.channels.remove(&conn_id);
        self.original_filters.remove(&conn_id);
    }

    /// Route an event to matching connections.
    pub fn deliver(&self, event: &SubscriptionEvent) -> (usize, usize) {
        let mut recipients = self.wildcard.clone();

        let category = event.category();
        if let Some(set) = self.by_category.get(&category) {
            for id in set { recipients.insert(*id); }
        }

        if let Some(community) = event.community() {
            let key = (category, community.to_string());
            if let Some(set) = self.by_community.get(&key) {
                for id in set { recipients.insert(*id); }
            }
        }

        if recipients.is_empty() { return (0, 0); }

        let msg: Message<BusPayload> = Message {
            wire_version: WIRE_VERSION,
            msg_id: Uuid::now_v7(),
            correlation_id: None,
            sender: Uuid::nil(),
            timestamp: Timestamp::now(self.epoch),
            payload: BusPayload::Event(event.clone()),
            security_level: SecurityLevel::Open,
            verified_sender_name: Some("server".to_string()),
            agent_type: None,
            community_scope: event.community().map(String::from),
        };
        let bytes = match encode_frame(&msg) {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "event_router: frame encode failed");
                return (0, 0);
            }
        };

        let mut delivered = 0usize;
        let mut dropped = 0usize;
        for conn_id in &recipients {
            if let Some(tx) = self.channels.get(conn_id) {
                if tx.try_send(bytes.clone()).is_ok() {
                    delivered += 1;
                } else {
                    dropped += 1;
                    debug!(conn_id, "event_router: channel full/closed");
                }
            }
        }

        (delivered, dropped)
    }

    fn index_filter(&mut self, conn_id: u64, filter: &SubscriptionFilter) {
        match (&filter.categories, &filter.community_scope) {
            (None, None) => { self.wildcard.insert(conn_id); }
            (None, Some(gov_key)) => {
                for cat in &ALL_CATEGORIES {
                    self.by_community.entry((*cat, gov_key.clone())).or_default().insert(conn_id);
                }
            }
            (Some(cats), None) => {
                for cat in cats { self.by_category.entry(*cat).or_default().insert(conn_id); }
            }
            (Some(cats), Some(gov_key)) => {
                for cat in cats {
                    self.by_community.entry((*cat, gov_key.clone())).or_default().insert(conn_id);
                }
            }
        }
    }

    fn deindex_connection(&mut self, conn_id: u64) {
        self.wildcard.remove(&conn_id);
        for set in self.by_category.values_mut() { set.remove(&conn_id); }
        for set in self.by_community.values_mut() { set.remove(&conn_id); }
        self.by_category.retain(|_, set| !set.is_empty());
        self.by_community.retain(|_, set| !set.is_empty());
    }
}

impl Default for EventRouter {
    fn default() -> Self { Self::new() }
}
