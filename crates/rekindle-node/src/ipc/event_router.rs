//! Sharded event router for O(1) per-event delivery at 100K+ agent scale.
//!
//! Architecture:
//! - **EventIndex** (centralized, `parking_lot::RwLock`): subscription metadata.
//!   Read-locked on deliver (hot path), write-locked on subscribe/unsubscribe (cold).
//! - **DeliveryStripe** (per-stripe, `parking_lot::RwLock`): channel handles.
//!   Each stripe owns channels for `conn_id % num_stripes`. Stripes are independently
//!   locked — deliveries to different stripes never contend.
//!
//! On subscribe: conn_id inserted into category/community index sets + stripe channel map.
//! On event arrival: 3 hash lookups produce exact recipient set, bucketed by stripe.
//! Delivery is parallelized across stripes via `std::thread::scope` behind
//! `tokio::task::block_in_place` for fan-out > 256 recipients.
//!
//! Serialization happens once per event, not once per recipient. Frame distribution
//! uses `bytes::Bytes` (atomic refcount clone, zero memcpy).
//!
//! The router is stored as `Arc<ShardedEventRouter>` on `ServerState`, NOT behind
//! an outer `RwLock`. Internal locking on `index` and `stripes[i]` provides the
//! necessary synchronization without blocking deliveries during subscribe/unsubscribe.
//!
//! # Constraints
//!
//! - `block_in_place` requires the multi-thread tokio runtime. It will panic on
//!   `current_thread` runtime or inside a `LocalSet`.
//! - The blocking thread pool (default cap 512) is used by `block_in_place` to
//!   hand off the worker's run queue. High event rates with large fan-out can
//!   pressure this pool. Monitor `tokio.blocking_threads` metrics.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use tokio::sync::mpsc;

use crate::ipc::message::SharedFrame;
use tracing::{info, warn};
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

/// Recipient count threshold below which sequential delivery is used.
/// Below this, thread spawn overhead (~5μs × N_stripes) exceeds the
/// parallelism benefit (~50ns × recipients / stripes).
const PARALLEL_THRESHOLD: usize = 256;

// Thread-local reusable HashSet for deduplication in `deliver()`.
// `clear()` resets without deallocating — capacity from the previous
// delivery is retained. After the first delivery to N recipients,
// the HashSet has capacity for N entries and never allocates again.
// Thread-local (not task-local) is correct because `deliver()` is
// synchronous — the thread doesn't change during the call.
thread_local! {
    static DEDUP_SET: RefCell<HashSet<u64>> = RefCell::new(HashSet::with_capacity(1024));
}

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

// ── Governance Key Interner ───────────────────────────────────────────

/// Maps governance key strings to compact u32 indices.
///
/// The set of communities is small (dozens) and stable. Interning
/// amortizes string hashing across all event deliveries — community
/// index lookups become `(u8, u32)` comparisons instead of string hashes.
struct GovKeyInterner {
    to_id: HashMap<String, u32>,
    #[allow(dead_code)] // Used for diagnostics/debugging
    to_str: Vec<String>,
}

impl GovKeyInterner {
    fn new() -> Self {
        Self {
            to_id: HashMap::new(),
            to_str: Vec::new(),
        }
    }

    fn intern(&mut self, key: &str) -> u32 {
        if let Some(&id) = self.to_id.get(key) {
            return id;
        }
        // Community count bounded by MAX_SUBSCRIBED_CONNECTIONS (100K) < u32::MAX.
        let id = u32::try_from(self.to_str.len())
            .expect("GovKeyInterner: community count exceeds u32::MAX");
        self.to_str.push(key.to_string());
        self.to_id.insert(key.to_string(), id);
        id
    }
}

// ── Index Key (reverse index) ─────────────────────────────────────────

/// Discriminated union of index positions for reverse-index cleanup.
///
/// When a connection disconnects, we use the reverse index
/// (`conn_index_keys`) to remove it from exactly the sets it appears in,
/// instead of scanning all sets.
#[derive(Clone, PartialEq, Eq, Hash)]
enum IndexKey {
    Wildcard,
    Category(EventCategory),
    Community(EventCategory, u32),
}

// ── Subscription Index ────────────────────────────────────────────────

/// Centralized subscription metadata — determines WHO receives each event.
///
/// Separated from the delivery mechanism (HOW they receive it) so that
/// the read lock is held only for recipient set construction, not for
/// the actual sends.
struct EventIndex {
    /// conn_ids subscribed to ALL events.
    wildcard: HashSet<u64>,
    /// EventCategory → conn_ids subscribed globally.
    by_category: HashMap<EventCategory, HashSet<u64>>,
    /// (EventCategory, interned_gov_key) → conn_ids.
    by_community: HashMap<(EventCategory, u32), HashSet<u64>>,
    /// conn_id → original filters (for unsubscribe matching).
    original_filters: HashMap<u64, Vec<SubscriptionFilter>>,
    /// Reverse index: conn_id → set of index keys for O(filters) cleanup.
    conn_index_keys: HashMap<u64, Vec<IndexKey>>,
    /// Governance key interner.
    interner: GovKeyInterner,
}

impl EventIndex {
    fn new() -> Self {
        Self {
            wildcard: HashSet::new(),
            by_category: HashMap::new(),
            by_community: HashMap::new(),
            original_filters: HashMap::new(),
            conn_index_keys: HashMap::new(),
            interner: GovKeyInterner::new(),
        }
    }

    fn index_filter(&mut self, conn_id: u64, filter: &SubscriptionFilter) {
        let keys = self.conn_index_keys.entry(conn_id).or_default();

        match (&filter.categories, &filter.community_scope) {
            (None, None) => {
                self.wildcard.insert(conn_id);
                keys.push(IndexKey::Wildcard);
            }
            (None, Some(gov_key)) => {
                let gov_id = self.interner.intern(gov_key);
                for cat in &ALL_CATEGORIES {
                    self.by_community
                        .entry((*cat, gov_id))
                        .or_default()
                        .insert(conn_id);
                    keys.push(IndexKey::Community(*cat, gov_id));
                }
            }
            (Some(cats), None) => {
                for cat in cats {
                    self.by_category.entry(*cat).or_default().insert(conn_id);
                    keys.push(IndexKey::Category(*cat));
                }
            }
            (Some(cats), Some(gov_key)) => {
                let gov_id = self.interner.intern(gov_key);
                for cat in cats {
                    self.by_community
                        .entry((*cat, gov_id))
                        .or_default()
                        .insert(conn_id);
                    keys.push(IndexKey::Community(*cat, gov_id));
                }
            }
        }
    }

    /// Remove a connection from all index sets in O(filters_for_this_conn).
    fn deindex_connection(&mut self, conn_id: u64) {
        self.wildcard.remove(&conn_id);

        if let Some(keys) = self.conn_index_keys.remove(&conn_id) {
            for key in keys {
                match key {
                    IndexKey::Wildcard => {} // Already removed above
                    IndexKey::Category(cat) => {
                        if let Some(set) = self.by_category.get_mut(&cat) {
                            set.remove(&conn_id);
                            if set.is_empty() {
                                self.by_category.remove(&cat);
                            }
                        }
                    }
                    IndexKey::Community(cat, gov_id) => {
                        let map_key = (cat, gov_id);
                        if let Some(set) = self.by_community.get_mut(&map_key) {
                            set.remove(&conn_id);
                            if set.is_empty() {
                                self.by_community.remove(&map_key);
                            }
                        }
                    }
                }
            }
        }

        self.original_filters.remove(&conn_id);
    }
}

// ── Delivery Stripe ───────────────────────────────────────────────────

/// One delivery stripe — owns the channel handles for a subset of connections.
///
/// Assignment: `conn_id % num_stripes`. Each stripe is independently locked.
/// A delivery to stripe K only contends with other operations on stripe K.
struct DeliveryStripe {
    channels: parking_lot::RwLock<HashMap<u64, mpsc::Sender<SharedFrame>>>,
}

impl DeliveryStripe {
    fn new() -> Self {
        Self {
            channels: parking_lot::RwLock::new(HashMap::new()),
        }
    }
}

// ── ShardedEventRouter ────────────────────────────────────────────────

/// Sharded event delivery system.
///
/// Stored as `Arc<ShardedEventRouter>` on `ServerState` — no outer RwLock.
/// Internal locking on `index` and each `stripes[i]` provides synchronization.
pub struct ShardedEventRouter {
    /// Centralized subscription index.
    index: parking_lot::RwLock<EventIndex>,
    /// Per-stripe delivery channels.
    stripes: Vec<DeliveryStripe>,
    /// Number of stripes (immutable after construction).
    num_stripes: usize,
    /// Monotonic epoch for timestamp generation.
    epoch: Instant,
    /// Pre-allocated server sender name — never re-allocated.
    server_name: Arc<str>,
}

impl ShardedEventRouter {
    /// Create a new sharded router.
    ///
    /// Stripe count matches available CPU parallelism, clamped to [2, 32].
    pub fn new() -> Self {
        let num_stripes = std::thread::available_parallelism()
            .map(|n| n.get().clamp(2, 32))
            .unwrap_or(4);

        let stripes = (0..num_stripes).map(|_| DeliveryStripe::new()).collect();

        Self {
            index: parking_lot::RwLock::new(EventIndex::new()),
            stripes,
            num_stripes,
            epoch: Instant::now(),
            server_name: Arc::from("server"),
        }
    }

    /// Register a connection's filters and delivery channel.
    pub fn subscribe(
        &self,
        conn_id: u64,
        filters: &[SubscriptionFilter],
        tx: mpsc::Sender<SharedFrame>,
    ) -> Result<usize, &'static str> {
        // Store channel in the appropriate stripe.
        let stripe_id = {
            // SAFETY: num_stripes is clamped to [2, 32] in new(). The modulo result
            // is always < 32, which fits in usize on all platforms (including 16-bit).
            #[allow(clippy::cast_possible_truncation)]
            { (conn_id % self.num_stripes as u64) as usize }
        };
        self.stripes[stripe_id].channels.write().insert(conn_id, tx);

        // Update the centralized index.
        let mut index = self.index.write();

        // Connection cap check.
        let is_new = !index.original_filters.contains_key(&conn_id);
        if is_new {
            let total_conns = index.original_filters.len();
            if total_conns >= MAX_SUBSCRIBED_CONNECTIONS {
                warn!(conn_id, max = MAX_SUBSCRIBED_CONNECTIONS, "max connections exceeded");
                // Remove the channel we just inserted.
                self.stripes[stripe_id].channels.write().remove(&conn_id);
                return Err("maximum subscribed connections exceeded");
            }
        }

        let existing_count = index.original_filters.get(&conn_id).map_or(0, Vec::len);
        let remaining = MAX_FILTERS_PER_CONNECTION.saturating_sub(existing_count);
        if filters.len() > remaining {
            return Err("maximum filters per connection exceeded");
        }

        for filter in filters {
            index.index_filter(conn_id, filter);
        }
        index.original_filters.entry(conn_id).or_default().extend_from_slice(filters);

        let total = existing_count + filters.len();
        info!(conn_id, new_filters = filters.len(), total_filters = total, "subscribed");
        Ok(total)
    }

    /// Remove matching filters for a connection.
    pub fn unsubscribe(&self, conn_id: u64, filters: &[SubscriptionFilter]) -> usize {
        let mut index = self.index.write();

        let remaining = if let Some(existing) = index.original_filters.get_mut(&conn_id) {
            for remove in filters {
                existing.retain(|f| {
                    f.categories != remove.categories || f.community_scope != remove.community_scope
                });
            }
            existing.clone()
        } else {
            return 0;
        };

        // Rebuild index entries for this connection from remaining filters.
        index.deindex_connection(conn_id);
        // Re-add the original_filters entry (deindex_connection removed it).
        index.original_filters.insert(conn_id, remaining.clone());
        for filter in &remaining {
            index.index_filter(conn_id, filter);
        }

        remaining.len()
    }

    /// Remove a connection entirely (disconnect cleanup).
    pub fn remove_connection(&self, conn_id: u64) {
        // Remove from stripe.
        let stripe_id = {
            // SAFETY: num_stripes is clamped to [2, 32] in new(). The modulo result
            // is always < 32, which fits in usize on all platforms (including 16-bit).
            #[allow(clippy::cast_possible_truncation)]
            { (conn_id % self.num_stripes as u64) as usize }
        };
        self.stripes[stripe_id].channels.write().remove(&conn_id);

        // Remove from index.
        self.index.write().deindex_connection(conn_id);
    }

    /// Route an event to matching connections.
    ///
    /// Returns (delivered, dropped). Serialization happens once. Frame
    /// distribution uses `Bytes` (zero-copy clone). For fan-out > 256,
    /// delivery is parallelized across stripes via `std::thread::scope`.
    pub fn deliver(&self, event: &SubscriptionEvent) -> (usize, usize) {
        let category = event.category();
        let community = event.community();

        // ── Phase 1: Build recipient set, bucketed by stripe ──────────
        //
        // Hold the index read lock only for recipient identification.
        // Dedup via `seen` HashSet — a conn_id in both wildcard and
        // by_category must not receive the frame twice.
        let stripe_buckets = {
            let index = self.index.read();

            // Estimate total recipients for pre-allocation.
            let estimated = index.wildcard.len()
                + index.by_category.get(&category).map_or(0, HashSet::len)
                + community
                    .and_then(|g| index.interner.to_id.get(g))
                    .and_then(|gid| index.by_community.get(&(category, *gid)))
                    .map_or(0, HashSet::len);

            if estimated == 0 {
                return (0, 0);
            }

            let mut buckets: Vec<Vec<u64>> = vec![Vec::new(); self.num_stripes];

            // Thread-local dedup set: clear() resets without deallocation.
            // After the first large delivery, capacity is retained for reuse.
            DEDUP_SET.with_borrow_mut(|seen| {
                seen.clear();

                for &conn_id in index.wildcard.iter()
                    .chain(index.by_category.get(&category).into_iter().flatten())
                    .chain(
                        community
                            .and_then(|g| index.interner.to_id.get(g))
                            .and_then(|gid| index.by_community.get(&(category, *gid)))
                            .into_iter()
                            .flatten(),
                    )
                {
                    if seen.insert(conn_id) {
                        #[allow(clippy::cast_possible_truncation)]
                        let stripe = (conn_id % self.num_stripes as u64) as usize;
                        buckets[stripe].push(conn_id);
                    }
                }
            });

            buckets
            // Index read lock released here — before any sends.
        };

        let total: usize = stripe_buckets.iter().map(Vec::len).sum();
        if total == 0 {
            return (0, 0);
        }

        // ── Phase 2: Serialize once ───────────────────────────────────
        let Some(frame) = self.serialize_event(event) else {
            return (0, 0);
        };

        // ── Phase 3: Deliver ──────────────────────────────────────────
        let active_stripes = stripe_buckets.iter().filter(|b| !b.is_empty()).count();

        if active_stripes <= 1 || total < PARALLEL_THRESHOLD {
            // Sequential fast path — not worth spawning threads.
            let mut delivered = 0usize;
            let mut dropped = 0usize;
            for (stripe_id, bucket) in stripe_buckets.iter().enumerate() {
                if bucket.is_empty() {
                    continue;
                }
                let guard = self.stripes[stripe_id].channels.read();
                for &conn_id in bucket {
                    if let Some(tx) = guard.get(&conn_id) {
                        if tx.try_send(frame.clone()).is_ok() {
                            delivered += 1;
                        } else {
                            dropped += 1;
                        }
                    }
                }
            }
            (delivered, dropped)
        } else {
            // Parallel path — one scoped thread per active stripe.
            //
            // block_in_place tells tokio to hand the worker's run queue to
            // a new thread (consuming one blocking pool slot), then runs
            // the closure on the current thread. The current thread never
            // changes, so parking_lot guards are safe (!Send is fine).
            //
            // Constraint: panics on current_thread runtime or inside LocalSet.
            tokio::task::block_in_place(|| {
                let delivered = AtomicUsize::new(0);
                let dropped = AtomicUsize::new(0);

                std::thread::scope(|s| {
                    for (stripe_id, bucket) in stripe_buckets.iter().enumerate() {
                        if bucket.is_empty() {
                            continue;
                        }

                        let frame = &frame;
                        let stripe = &self.stripes[stripe_id];
                        let del = &delivered;
                        let drp = &dropped;

                        s.spawn(move || {
                            let guard = stripe.channels.read();
                            for &conn_id in bucket {
                                if let Some(tx) = guard.get(&conn_id) {
                                    if tx.try_send(frame.clone()).is_ok() {
                                        del.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        drp.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                        });
                    }
                }); // All stripe threads join here.

                (
                    delivered.load(Ordering::Relaxed),
                    dropped.load(Ordering::Relaxed),
                )
            })
        }
    }

    /// Serialize an event into a wire-ready shared frame.
    ///
    /// Returns `SharedFrame` (backed by `Arc<[u8]>`). This is serialized once;
    /// all subsequent fan-out clones are `Arc::clone` (~5ns, no vtable dispatch).
    fn serialize_event(&self, event: &SubscriptionEvent) -> Option<SharedFrame> {
        let msg: Message<BusPayload> = Message {
            wire_version: WIRE_VERSION,
            msg_id: Uuid::now_v7(),
            correlation_id: None,
            sender: Uuid::nil(),
            security_level: SecurityLevel::Open,
            timestamp: Timestamp::now(self.epoch),
            payload: BusPayload::Event(event.clone()),
            verified_sender_name: Some(Arc::clone(&self.server_name)),
            agent_type: None,
            community_scope: event.community().map(String::from),
        };
        match encode_frame(&msg) {
            Ok(bytes) => Some(SharedFrame::from_bytes(&bytes)),
            Err(e) => {
                warn!(error = %e, "event_router: frame encode failed");
                None
            }
        }
    }
}

static_assertions::assert_impl_all!(ShardedEventRouter: Send, Sync);

impl Default for ShardedEventRouter {
    fn default() -> Self {
        Self::new()
    }
}
