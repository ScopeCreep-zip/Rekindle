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
    ///
    /// Translates each `SubscriptionFilter` into index insertions for O(1) lookup.
    /// Filters are additive — subsequent calls for the same conn_id append.
    ///
    /// Returns the total filter count for this connection, or an error string.
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

        // Check filter limit before mutating
        let existing_count = self.original_filters.get(&conn_id).map_or(0, Vec::len);
        let remaining = MAX_FILTERS_PER_CONNECTION.saturating_sub(existing_count);
        if filters.len() > remaining {
            return Err("maximum filters per connection exceeded");
        }

        self.channels.insert(conn_id, tx);

        // Index each filter for O(1) lookup
        for filter in filters {
            self.index_filter(conn_id, filter);
        }

        // Store original filters for unsubscribe matching
        self.original_filters.entry(conn_id).or_default().extend_from_slice(filters);
        let total = existing_count + filters.len();

        info!(
            conn_id,
            new_filters = filters.len(),
            total_filters = total,
            index_entries = self.index_size(),
            "event_router: subscribed"
        );

        Ok(total)
    }

    /// Remove matching filters for a connection. Updates all index sets.
    ///
    /// Returns remaining filter count. If zero, connection receives no events
    /// but stays registered (channel preserved for re-subscribe).
    pub fn unsubscribe(&mut self, conn_id: u64, filters: &[SubscriptionFilter]) -> usize {
        // Remove matching filters, clone remaining, then rebuild index
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

        // Rebuild index for this connection from remaining filters
        self.deindex_connection(conn_id);
        for filter in &remaining {
            self.index_filter(conn_id, filter);
        }

        let count = remaining.len();
        info!(conn_id, removed = filters.len(), remaining = count, "event_router: unsubscribed");
        count
    }

    /// Remove a connection entirely. Cleans all index sets + channel + filters.
    pub fn remove_connection(&mut self, conn_id: u64) {
        self.deindex_connection(conn_id);
        self.channels.remove(&conn_id);
        self.original_filters.remove(&conn_id);
        info!(conn_id, "event_router: connection removed");
    }

    /// Route an event to matching connections.
    ///
    /// 3 hash lookups → exact recipient set → serialize once → send to each.
    /// Returns `(delivered, dropped)` counts.
    pub fn deliver(&self, event: &SubscriptionEvent) -> (usize, usize) {
        // Collect recipients via index lookups
        let mut recipients = self.wildcard.clone();

        let category = event.category();
        if let Some(set) = self.by_category.get(&category) {
            for id in set {
                recipients.insert(*id);
            }
        }

        if let Some(community) = event.community() {
            let key = (category, community.to_string());
            if let Some(set) = self.by_community.get(&key) {
                for id in set {
                    recipients.insert(*id);
                }
            }
        }

        if recipients.is_empty() {
            return (0, 0);
        }

        // Build a proper Message<BusPayload> envelope and encode via postcard.
        // This is what the client's decode_frame() expects.
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

        // Deliver to each recipient
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

        debug!(
            category = ?category,
            recipients = recipients.len(),
            delivered,
            dropped,
            "event_router: delivered"
        );

        (delivered, dropped)
    }

    /// Number of subscribed connections.
    pub fn connection_count(&self) -> usize {
        self.channels.len()
    }

    /// Total index entries across all sets (for diagnostics).
    pub fn index_size(&self) -> usize {
        self.wildcard.len()
            + self.by_category.values().map(HashSet::len).sum::<usize>()
            + self.by_community.values().map(HashSet::len).sum::<usize>()
    }

    // ── Internal index management ──────────────────────────────────────

    /// Insert conn_id into the correct index sets based on a filter.
    fn index_filter(&mut self, conn_id: u64, filter: &SubscriptionFilter) {
        match (&filter.categories, &filter.community_scope) {
            // Wildcard: all categories, all communities
            (None, None) => {
                self.wildcard.insert(conn_id);
            }
            // All categories for a specific community
            (None, Some(gov_key)) => {
                for cat in &ALL_CATEGORIES {
                    self.by_community
                        .entry((*cat, gov_key.clone()))
                        .or_default()
                        .insert(conn_id);
                }
            }
            // Specific categories, all communities
            (Some(cats), None) => {
                for cat in cats {
                    self.by_category.entry(*cat).or_default().insert(conn_id);
                }
            }
            // Specific categories for a specific community
            (Some(cats), Some(gov_key)) => {
                for cat in cats {
                    self.by_community
                        .entry((*cat, gov_key.clone()))
                        .or_default()
                        .insert(conn_id);
                }
            }
        }
    }

    /// Remove conn_id from ALL index sets. Used before re-indexing after unsubscribe.
    fn deindex_connection(&mut self, conn_id: u64) {
        self.wildcard.remove(&conn_id);
        for set in self.by_category.values_mut() {
            set.remove(&conn_id);
        }
        for set in self.by_community.values_mut() {
            set.remove(&conn_id);
        }
        // Clean up empty sets to prevent unbounded growth
        self.by_category.retain(|_, set| !set.is_empty());
        self.by_community.retain(|_, set| !set.is_empty());
    }
}

impl Default for EventRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::subscription_events::{
        ChannelMessageEvent, FriendEvent, SubscriptionEvent,
    };

    fn make_tx() -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        mpsc::channel(64)
    }

    #[test]
    fn wildcard_receives_all_events() {
        let mut router = EventRouter::new();
        let (tx, mut rx) = make_tx();
        router.subscribe(1, &[SubscriptionFilter::all()], tx).unwrap();

        let event = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
            from_key: "abc".into(),
            display_name: "alice".into(),
            message: "hi".into(),
        });
        let (delivered, _) = router.deliver(&event);
        assert_eq!(delivered, 1);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn category_filter_matches_correct_events() {
        let mut router = EventRouter::new();
        let (tx, mut rx) = make_tx();
        router.subscribe(1, &[SubscriptionFilter::categories(vec![EventCategory::Friend])], tx).unwrap();

        // Friend event: should match
        let friend_event = SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: "x".into() });
        assert_eq!(router.deliver(&friend_event).0, 1);
        assert!(rx.try_recv().is_ok());

        // Channel event: should not match
        let channel_event = SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            community: "gov1".into(), channel: "gen".into(), message_id: "m1".into(),
            sender_pseudonym: "s".into(), sequence: 0, timestamp: 0,
            body: None, reply_to_sequence: None,
        });
        assert_eq!(router.deliver(&channel_event).0, 0);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn community_scope_restricts_delivery() {
        let mut router = EventRouter::new();
        let (tx, mut rx) = make_tx();
        router.subscribe(1, &[SubscriptionFilter::community("gov1".into())], tx).unwrap();

        // Event for gov1: should match
        let e1 = SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            community: "gov1".into(), channel: "gen".into(), message_id: "m1".into(),
            sender_pseudonym: "s".into(), sequence: 0, timestamp: 0,
            body: None, reply_to_sequence: None,
        });
        assert_eq!(router.deliver(&e1).0, 1);
        assert!(rx.try_recv().is_ok());

        // Event for gov2: should not match
        let e2 = SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            community: "gov2".into(), channel: "gen".into(), message_id: "m2".into(),
            sender_pseudonym: "s".into(), sequence: 0, timestamp: 0,
            body: None, reply_to_sequence: None,
        });
        assert_eq!(router.deliver(&e2).0, 0);
        assert!(rx.try_recv().is_err());

        // Global event (no community): should match (global events pass community filters)
        let e3 = SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: "x".into() });
        assert_eq!(router.deliver(&e3).0, 1);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn no_subscription_receives_nothing() {
        let router = EventRouter::new();
        let event = SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: "x".into() });
        assert_eq!(router.deliver(&event), (0, 0));
    }

    #[test]
    fn unsubscribe_removes_delivery() {
        let mut router = EventRouter::new();
        let (tx, mut rx) = make_tx();
        let filter = SubscriptionFilter::all();
        router.subscribe(1, &[filter.clone()], tx).unwrap();

        let event = SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: "x".into() });
        assert_eq!(router.deliver(&event).0, 1);
        assert!(rx.try_recv().is_ok());

        router.unsubscribe(1, &[filter]);
        assert_eq!(router.deliver(&event).0, 0);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn remove_connection_cleans_everything() {
        let mut router = EventRouter::new();
        let (tx, _rx) = make_tx();
        router.subscribe(1, &[SubscriptionFilter::all()], tx).unwrap();
        assert_eq!(router.connection_count(), 1);

        router.remove_connection(1);
        assert_eq!(router.connection_count(), 0);
        assert_eq!(router.index_size(), 0);
    }

    #[test]
    fn dedup_recipients_across_index_sets() {
        let mut router = EventRouter::new();
        let (tx, mut rx) = make_tx();
        // Subscribe with overlapping filters: wildcard + specific category
        router.subscribe(1, &[
            SubscriptionFilter::all(),
            SubscriptionFilter::categories(vec![EventCategory::Friend]),
        ], tx).unwrap();

        let event = SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: "x".into() });
        let (delivered, _) = router.deliver(&event);
        // conn_id 1 appears in both wildcard and by_category — should only deliver once
        assert_eq!(delivered, 1);
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err()); // no duplicate
    }

    #[test]
    fn filter_limit_enforced() {
        let mut router = EventRouter::new();
        let (tx, _rx) = make_tx();
        let filters: Vec<SubscriptionFilter> = (0..MAX_FILTERS_PER_CONNECTION + 1)
            .map(|_| SubscriptionFilter::all())
            .collect();
        let result = router.subscribe(1, &filters, tx);
        assert!(result.is_err());
    }
}
