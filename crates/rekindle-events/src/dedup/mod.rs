//! Cross-tier event deduplication via Blake3 content hashing.
//!
//! Every `SubscriptionEvent` is Blake3-hashed before emission. The hash
//! covers semantically meaningful fields — not pathway-specific metadata
//! like arrival timestamp or source tier. A single bounded `HashSet<[u8; 32]>`
//! rejects duplicates regardless of whether the event arrived via DHT watch,
//! gossip mesh, or periodic poll.
//!
//! At 100K+ agent scale, this ensures the IPC bus carries only unique events.

use std::collections::{HashSet, VecDeque};
use std::time::Instant;

use tracing::{debug, trace};

use rekindle_types::subscription_events::SubscriptionEvent;

// Per-event-family Blake3 content hashers, split out by domain for
// line-count. CRITICAL INVARIANT: these hashers are wire-visible dedup
// state — do NOT reorder any `h.update()` call or alter any tag string
// across the `hash_*` modules below. Doing so changes the digest for
// events already in flight and breaks cross-tier dedup for peers running
// different versions.
mod hash_crypto_voice;
mod hash_membership;
mod hash_messaging;
mod hash_social;
mod hash_system;

use hash_crypto_voice::{hash_crypto, hash_voice};
use hash_membership::{hash_friend, hash_membership};
use hash_messaging::{hash_channel_message, hash_presence, hash_typing};
use hash_social::{hash_governance, hash_social};
use hash_system::{hash_network, hash_notification, hash_system};

/// Content-addressed event deduplication using Blake3 digests.
///
/// Events are hashed by their canonical content (not pathway metadata).
/// The digest set is bounded by capacity with FIFO eviction. Expired
/// entries are also evicted on periodic cleanup.
pub struct EventDedup {
    /// Set of Blake3 digests for events already emitted.
    digests: HashSet<[u8; 32]>,
    /// FIFO order for eviction when at capacity.
    order: VecDeque<([u8; 32], Instant)>,
    /// Maximum number of digests to retain.
    capacity: usize,
    /// TTL for digest entries — events older than this are evictable.
    ttl_secs: u64,
    /// Count of duplicates suppressed (diagnostic).
    suppressed_count: u64,
}

impl EventDedup {
    /// Create a new dedup cache with the given capacity and TTL.
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            digests: HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
            ttl_secs,
            suppressed_count: 0,
        }
    }

    /// Check if this event is new (should emit) or duplicate (suppress).
    ///
    /// Returns `true` if the event is new and has been recorded.
    /// Returns `false` if the event is a duplicate.
    pub fn check(&mut self, event: &SubscriptionEvent) -> bool {
        // UnreadChanged is always emitted — it's a computed aggregate, not a network event
        if matches!(event, SubscriptionEvent::UnreadChanged { .. }) {
            return true;
        }

        let digest = hash_event(event);

        if self.digests.contains(&digest) {
            self.suppressed_count += 1;
            trace!(
                suppressed = self.suppressed_count,
                "dedup: duplicate suppressed"
            );
            return false;
        }

        // Evict oldest if at capacity
        if self.digests.len() >= self.capacity {
            if let Some((old_digest, _)) = self.order.pop_front() {
                self.digests.remove(&old_digest);
            }
        }

        self.digests.insert(digest);
        self.order.push_back((digest, Instant::now()));
        true
    }

    /// Evict entries older than TTL.
    pub fn evict_expired(&mut self) {
        let cutoff = self.ttl_secs;
        let mut evicted = 0u32;
        while let Some((digest, inserted)) = self.order.front() {
            if inserted.elapsed().as_secs() > cutoff {
                let d = *digest;
                self.order.pop_front();
                self.digests.remove(&d);
                evicted += 1;
            } else {
                break; // ordered by insertion time, so all remaining are newer
            }
        }
        if evicted > 0 {
            debug!(
                evicted,
                remaining = self.digests.len(),
                "dedup: expired entries evicted"
            );
        }
    }

    /// Number of active digest entries.
    pub fn len(&self) -> usize {
        self.digests.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.digests.is_empty()
    }

    /// Total duplicates suppressed since creation.
    pub fn suppressed_count(&self) -> u64 {
        self.suppressed_count
    }
}

impl Default for EventDedup {
    fn default() -> Self {
        Self::new(10_000, 300) // 10K entries, 5-minute TTL
    }
}

// ── Blake3 content hashing ─────────────────────────────────────────────

/// Compute the Blake3 digest of an event's canonical content.
///
/// The hash covers semantically meaningful fields only. Pathway-specific
/// metadata (which tier delivered it, arrival timestamp) is excluded so
/// the same event from different tiers produces the same digest.
fn hash_event(event: &SubscriptionEvent) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();

    // Tag with the outer discriminant to prevent cross-type collisions
    hasher.update(event_discriminant_tag(event).as_bytes());
    hasher.update(b"|");

    match event {
        SubscriptionEvent::ChannelMessage(msg) => hash_channel_message(&mut hasher, msg),
        SubscriptionEvent::Typing(t) => hash_typing(&mut hasher, t),
        SubscriptionEvent::Presence(p) => hash_presence(&mut hasher, p),
        SubscriptionEvent::Membership(m) => hash_membership(&mut hasher, m),
        SubscriptionEvent::Friend(f) => hash_friend(&mut hasher, f),
        SubscriptionEvent::Crypto(c) => hash_crypto(&mut hasher, c),
        SubscriptionEvent::Voice(v) => hash_voice(&mut hasher, v),
        SubscriptionEvent::Governance(g) => hash_governance(&mut hasher, g),
        SubscriptionEvent::Social(s) => hash_social(&mut hasher, s),
        SubscriptionEvent::Notification(n) => hash_notification(&mut hasher, n),
        SubscriptionEvent::Network(n) => hash_network(&mut hasher, n),
        SubscriptionEvent::System(s) => hash_system(&mut hasher, s),
        SubscriptionEvent::UnreadChanged { .. } => {
            // Never reaches here — check() returns true early for UnreadChanged
            hasher.update(b"unread");
        }
    }

    *hasher.finalize().as_bytes()
}

fn event_discriminant_tag(event: &SubscriptionEvent) -> &'static str {
    match event {
        SubscriptionEvent::ChannelMessage(_) => "ch",
        SubscriptionEvent::Typing(_) => "ty",
        SubscriptionEvent::Presence(_) => "pr",
        SubscriptionEvent::Membership(_) => "mb",
        SubscriptionEvent::Friend(_) => "fr",
        SubscriptionEvent::Crypto(_) => "cr",
        SubscriptionEvent::Voice(_) => "vo",
        SubscriptionEvent::Governance(_) => "go",
        SubscriptionEvent::Social(_) => "so",
        SubscriptionEvent::Notification(_) => "nt",
        SubscriptionEvent::Network(_) => "ne",
        SubscriptionEvent::System(_) => "sy",
        SubscriptionEvent::UnreadChanged { .. } => "ur",
    }
}

#[cfg(test)]
mod tests;
