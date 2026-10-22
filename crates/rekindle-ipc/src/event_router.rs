//! Inverted-index subscription filters for the bus server.
//!
//! On subscribe: conn_id inserted into category/community index sets. Each
//! subscribed connection's stream task (`server::stream`) asks
//! [`EventRouter::matches`] for every journaled event: three set lookups,
//! no per-filter scanning. The media fan-out reads the subscriber set.

use std::collections::{HashMap, HashSet};

use tokio::sync::mpsc;
use tracing::{info, warn};

use rekindle_types::subscription_events::{
    EventCategory, SubscriptionEvent, SubscriptionFilter, MAX_FILTERS_PER_CONNECTION,
};

/// Maximum subscribable connections. Bounds memory at scale.
const MAX_SUBSCRIBED_CONNECTIONS: usize = 100_000;

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
}

impl EventRouter {
    pub fn new() -> Self {
        Self {
            wildcard: HashSet::new(),
            by_category: HashMap::new(),
            by_community: HashMap::new(),
            channels: HashMap::new(),
            original_filters: HashMap::new(),
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
        if self.channels.len() >= MAX_SUBSCRIBED_CONNECTIONS
            && !self.channels.contains_key(&conn_id)
        {
            warn!(
                conn_id,
                max = MAX_SUBSCRIBED_CONNECTIONS,
                "event_router: max connections exceeded"
            );
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
        self.original_filters
            .entry(conn_id)
            .or_default()
            .extend_from_slice(filters);
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
        info!(
            conn_id,
            removed = filters.len(),
            remaining = count,
            "event_router: unsubscribed"
        );
        count
    }

    /// Remove a connection entirely. Cleans all index sets + channel + filters.
    pub fn remove_connection(&mut self, conn_id: u64) {
        self.deindex_connection(conn_id);
        self.channels.remove(&conn_id);
        self.original_filters.remove(&conn_id);
        info!(conn_id, "event_router: connection removed");
    }

    /// Whether `conn_id`'s filters admit `event`: three set lookups.
    /// A global event (no community) also reaches the connection's
    /// community-scoped filters of its category.
    #[must_use]
    pub fn matches(&self, conn_id: u64, event: &SubscriptionEvent) -> bool {
        if self.wildcard.contains(&conn_id) {
            return true;
        }
        let category = event.category();
        if self
            .by_category
            .get(&category)
            .is_some_and(|set| set.contains(&conn_id))
        {
            return true;
        }
        match event.community() {
            Some(community) => self
                .by_community
                .get(&(category, community.to_string()))
                .is_some_and(|set| set.contains(&conn_id)),
            None => self
                .by_community
                .iter()
                .any(|((cat, _), set)| *cat == category && set.contains(&conn_id)),
        }
    }

    /// Number of subscribed connections.
    pub fn connection_count(&self) -> usize {
        self.channels.len()
    }

    /// Connection ids of every subscribed connection.
    ///
    /// Used by the media fan-out (`BusPayload::Media`), which delivers to
    /// subscribers **directly** — bypassing this router's per-category dedup
    /// and journal — so it needs the subscriber set without an event to
    /// classify. A media consumer is, by construction, a subscriber (it asked
    /// for the community's events), so the subscriber set is the media
    /// audience.
    pub fn subscribed_conn_ids(&self) -> Vec<u64> {
        self.channels.keys().copied().collect()
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
                for cat in &EventCategory::ALL {
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
    use rekindle_types::subscription_events::{ChannelMessageEvent, FriendEvent};

    fn make_tx() -> mpsc::Sender<Vec<u8>> {
        mpsc::channel(64).0
    }

    fn friend() -> SubscriptionEvent {
        SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: "x".into(),
        })
    }

    fn channel_message(community: &str) -> SubscriptionEvent {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            community: community.into(),
            channel: "gen".into(),
            message_id: "m1".into(),
            sender_pseudonym: "s".into(),
            sequence: 0,
            timestamp: 0,
            body: None,
            reply_to_message_id: None,
        })
    }

    #[test]
    fn wildcard_matches_every_event() {
        let mut router = EventRouter::new();
        router
            .subscribe(1, &[SubscriptionFilter::all()], make_tx())
            .unwrap();
        assert!(router.matches(1, &friend()));
        assert!(router.matches(1, &channel_message("gov1")));
        assert!(!router.matches(2, &friend()));
    }

    #[test]
    fn category_filter_matches_its_category_only() {
        let mut router = EventRouter::new();
        router
            .subscribe(
                1,
                &[SubscriptionFilter::categories(vec![EventCategory::Friend])],
                make_tx(),
            )
            .unwrap();
        assert!(router.matches(1, &friend()));
        assert!(!router.matches(1, &channel_message("gov1")));
    }

    #[test]
    fn community_scope_restricts_matching() {
        let mut router = EventRouter::new();
        router
            .subscribe(
                1,
                &[SubscriptionFilter::community("gov1".into())],
                make_tx(),
            )
            .unwrap();
        assert!(router.matches(1, &channel_message("gov1")));
        assert!(!router.matches(1, &channel_message("gov2")));
        // Global events pass community filters.
        assert!(router.matches(1, &friend()));
    }

    /// A community-scoped subscriber still receives device-wide events of
    /// every category. Calls and notifications were missing from the
    /// category list this expands, so such a subscriber never rang.
    #[test]
    fn community_scope_receives_calls_and_notifications() {
        use rekindle_types::subscription_events::{CallEvent, NotificationEvent};
        let mut router = EventRouter::new();
        router
            .subscribe(
                1,
                &[SubscriptionFilter::community("gov1".into())],
                make_tx(),
            )
            .unwrap();
        assert!(router.matches(
            1,
            &SubscriptionEvent::Call(CallEvent::Ringing {
                call_id: "c".into(),
            })
        ));
        assert!(router.matches(
            1,
            &SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
                title: "t".into(),
                body: "b".into(),
            })
        ));
    }

    #[test]
    fn unsubscribe_removes_matching() {
        let mut router = EventRouter::new();
        let filter = SubscriptionFilter::all();
        router
            .subscribe(1, std::slice::from_ref(&filter), make_tx())
            .unwrap();
        assert!(router.matches(1, &friend()));
        router.unsubscribe(1, &[filter]);
        assert!(!router.matches(1, &friend()));
    }

    #[test]
    fn remove_connection_cleans_everything() {
        let mut router = EventRouter::new();
        router
            .subscribe(1, &[SubscriptionFilter::all()], make_tx())
            .unwrap();
        assert_eq!(router.connection_count(), 1);
        router.remove_connection(1);
        assert_eq!(router.connection_count(), 0);
        assert_eq!(router.index_size(), 0);
    }

    #[test]
    fn filter_limit_enforced() {
        let mut router = EventRouter::new();
        let filters: Vec<SubscriptionFilter> = (0..=MAX_FILTERS_PER_CONNECTION)
            .map(|_| SubscriptionFilter::all())
            .collect();
        assert!(router.subscribe(1, &filters, make_tx()).is_err());
    }
}
