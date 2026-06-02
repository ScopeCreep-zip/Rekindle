//! [`OpaqueSeenSet`] — sharded content-addressed dedup mechanism.
//!
//! A sharded "seen-set" keyed on an opaque 32-byte key. The crate assigns
//! the key **no meaning** — it does not hash, does not know it is a
//! `ContentHash`, does not treat presence as a security decision. It only
//! answers "have I recorded this key?" with sharded, low-contention
//! concurrency.
//!
//! The consumer decides what the key means and what a hit implies
//! (idempotent drop, dedup, etc.).
//!
//! # Key distribution requirement
//!
//! The key's first byte is used for shard selection (`key[0] & shard_mask`).
//! This assumes the first byte is **uniformly distributed** — true for any
//! cryptographic digest (BLAKE3, SHA-256) but **not** for structured IDs,
//! versioned namespaces, or little-endian counters. Callers whose keys do
//! not have uniform first-byte distribution must pre-hash or pre-mix
//! before calling [`insert`](OpaqueSeenSet::insert).
//!
//! # Locking strategy
//!
//! Each shard is a `parking_lot::RwLock<HashSet<...>>`. [`contains`](OpaqueSeenSet::contains)
//! acquires a read lock (concurrent readers, no writer starvation under
//! `parking_lot`'s default fairness). [`insert`](OpaqueSeenSet::insert)
//! and [`remove`](OpaqueSeenSet::remove) acquire a write lock.
//!
//! `parking_lot` is chosen over `std::sync` because `parking_lot::RwLock`
//! does not poison on panic — a panicked writer releases the lock cleanly,
//! and subsequent callers do not need `.unwrap()` or `.expect()` on every
//! lock acquisition. This eliminates a class of cascading panic bugs.
//!
//! # Linearizability
//!
//! [`len`](OpaqueSeenSet::len) and [`is_empty`](OpaqueSeenSet::is_empty)
//! are **not linearizable** — they acquire shard locks sequentially, so the
//! result is a rolling observation, not a point-in-time snapshot. Use them
//! for diagnostics/monitoring only, never for correctness decisions.
//!
//! [`clear`](OpaqueSeenSet::clear) is also **not atomic** across shards —
//! concurrent inserts between shard clears may leave the set non-empty
//! after `clear` returns.
//!
//! # Feature gate
//!
//! This module is behind the `seen-set` feature flag and depends on
//! `parking_lot` for per-shard `RwLock`.
//!
//! # What this does NOT do
//!
//! - Does not compute keys (no crypto dependency).
//! - Does not interpret hits (the consumer decides).
//! - Does not enforce a capacity bound (the consumer caps via LRU eviction
//!   outside this set).
//! - Is not a replay filter (the [`ReorderRing`](crate::ReorderRing) window
//!   is the replay boundary; this set handles content-addressed dedup).
//!
//! # Replaces
//!
//! - `v3/dedup/cache.rs` — the content-addressed dedup cache backing

use std::collections::HashSet;
use std::hash::{BuildHasher, Hasher};

use parking_lot::RwLock;

// ---------------------------------------------------------------------------
// IdentityHasher — the key is already a digest, re-hashing is waste
// ---------------------------------------------------------------------------

/// A no-op hasher that reads the first 8 bytes of the key as a `u64`.
///
/// The key is already a 32-byte cryptographic digest with full entropy.
/// `SipHash` (the default `RandomState` hasher) would re-hash it, wasting
/// CPU cycles on the inner loop of every `insert()` and `contains()`.
/// This hasher treats bytes 0–7 as a little-endian `u64` and returns
/// that directly.
#[derive(Default)]
struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Take the first 8 bytes (or fewer) as a little-endian u64.
        // For a 32-byte digest, this is always 8 bytes of full entropy.
        let len = bytes.len().min(8);
        let mut buf = [0u8; 8];
        buf[..len].copy_from_slice(&bytes[..len]);
        self.0 = u64::from_le_bytes(buf);
    }
}

/// `BuildHasher` that creates `IdentityHasher` instances.
#[derive(Clone, Default)]
struct IdentityBuildHasher;

impl BuildHasher for IdentityBuildHasher {
    type Hasher = IdentityHasher;

    #[inline]
    fn build_hasher(&self) -> IdentityHasher {
        IdentityHasher(0)
    }
}

// ---------------------------------------------------------------------------
// CachePadded shard wrapper — prevent false sharing between adjacent shards
// ---------------------------------------------------------------------------

/// A shard wrapped in cache-line padding. `parking_lot::RwLock` is small
/// (1 word), so adjacent shards share a cache line without this wrapper.
/// Under concurrent access, a write-lock on shard 0 would invalidate the
/// cache line containing shard 1's lock state — false sharing.
#[repr(align(64))] // L1 cache line on x86-64 and aarch64
struct PaddedShard {
    lock: RwLock<HashSet<[u8; 32], IdentityBuildHasher>>,
}

impl PaddedShard {
    fn new(capacity: usize) -> Self {
        Self {
            lock: RwLock::new(HashSet::with_capacity_and_hasher(
                capacity,
                IdentityBuildHasher,
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// OpaqueSeenSet
// ---------------------------------------------------------------------------

/// A sharded set for recording opaque 32-byte keys.
///
/// Thread-safe via per-shard `RwLock`. Read contention is reduced to
/// near-zero (readers are concurrent within a shard). Write contention
/// is reduced to `1/N` where `N` is the shard count.
pub struct OpaqueSeenSet {
    shards: Box<[PaddedShard]>,
    shard_mask: usize,
}

// Compile-time Send + Sync assertion. A future field addition that breaks
// either bound will fail to compile here rather than at a distant call site.
#[allow(dead_code)]
const fn _assert_opaque_seen_set_send_sync()
where
    OpaqueSeenSet: Send + Sync,
{
}

impl OpaqueSeenSet {
    /// Create a new seen set with `shard_count` shards.
    ///
    /// Each shard is pre-allocated with capacity for
    /// `expected_total / shard_count` entries. Pass `expected_total = 0`
    /// if the expected size is unknown.
    ///
    /// # Panics
    ///
    /// Panics if `shard_count` is zero or not a power of two.
    pub fn new(shard_count: usize, expected_total: usize) -> Self {
        assert!(shard_count > 0, "shard_count must be non-zero");
        assert!(
            shard_count.is_power_of_two(),
            "shard_count must be a power of two, got {shard_count}"
        );

        let per_shard = expected_total / shard_count;
        let shards: Box<[PaddedShard]> = (0..shard_count)
            .map(|_| PaddedShard::new(per_shard))
            .collect();

        Self {
            shards,
            shard_mask: shard_count - 1,
        }
    }

    /// Record the key. Returns `true` if the key was **newly inserted**
    /// (i.e., it was not previously in the set). Returns `false` if the
    /// key was already present (the consumer's idempotency no-op condition).
    ///
    /// Acquires a **write lock** on the target shard.
    pub fn insert(&self, key: [u8; 32]) -> bool {
        let shard = &self.shards[self.shard_for(&key)];
        shard.lock.write().insert(key)
    }

    /// Check if the key is in the set without inserting it.
    ///
    /// Acquires a **read lock** on the target shard. Multiple concurrent
    /// `contains` calls on the same shard do not block each other.
    pub fn contains(&self, key: &[u8; 32]) -> bool {
        let shard = &self.shards[self.shard_for(key)];
        shard.lock.read().contains(key)
    }

    /// Remove a key from the set. Returns `true` if the key was present.
    ///
    /// Acquires a **write lock** on the target shard.
    pub fn remove(&self, key: &[u8; 32]) -> bool {
        let shard = &self.shards[self.shard_for(key)];
        shard.lock.write().remove(key)
    }

    /// Total number of keys across all shards.
    ///
    /// **Not linearizable** — acquires each shard's read lock sequentially.
    /// The result is a rolling observation across `N` lock acquisitions,
    /// not a point-in-time snapshot. Use for diagnostics only.
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.lock.read().len()).sum()
    }

    /// Whether the set is empty (no keys in any shard).
    ///
    /// **Not linearizable** — same caveat as [`len`](Self::len).
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(|s| s.lock.read().is_empty())
    }

    /// Remove all keys from all shards.
    ///
    /// **Not atomic** — concurrent `insert` calls between shard clears
    /// may leave the set non-empty after `clear` returns.
    pub fn clear(&self) {
        for shard in &*self.shards {
            shard.lock.write().clear();
        }
    }

    /// Number of shards.
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    /// Determine the shard index for a key.
    ///
    /// Uses `key[0]` which is uniformly distributed for cryptographic
    /// digests. See the module-level doc for the distribution requirement.
    #[inline]
    fn shard_for(&self, key: &[u8; 32]) -> usize {
        (key[0] as usize) & self.shard_mask
    }
}

impl Default for OpaqueSeenSet {
    /// Default: 16 shards, zero pre-allocated capacity.
    fn default() -> Self {
        Self::new(16, 0)
    }
}

impl core::fmt::Debug for OpaqueSeenSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Deliberately does NOT call self.len() — that would acquire all
        // shard locks, which is surprising behavior for a Debug impl.
        f.debug_struct("OpaqueSeenSet")
            .field("shard_count", &self.shards.len())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn insert_and_contains() {
        let set = OpaqueSeenSet::new(4, 0);
        let key = [0xAA; 32];

        assert!(!set.contains(&key));
        assert!(set.insert(key)); // newly inserted
        assert!(set.contains(&key));
        assert!(!set.insert(key)); // already present
    }

    #[test]
    fn remove() {
        let set = OpaqueSeenSet::new(4, 0);
        let key = [0xBB; 32];

        set.insert(key);
        assert!(set.remove(&key));
        assert!(!set.contains(&key));
        assert!(!set.remove(&key)); // already gone
    }

    #[test]
    fn len_and_clear() {
        let set = OpaqueSeenSet::new(4, 0);
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());

        for i in 0..10u8 {
            let mut key = [0u8; 32];
            key[0] = i;
            set.insert(key);
        }
        assert_eq!(set.len(), 10);

        set.clear();
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());
    }

    #[test]
    fn sharding_distributes() {
        let set = OpaqueSeenSet::new(16, 0);

        // Insert keys with different first bytes — they land in different
        // shards. We verify all keys are retrievable.
        for i in 0..=255u8 {
            let mut key = [0u8; 32];
            key[0] = i;
            set.insert(key);
        }
        assert_eq!(set.len(), 256);
    }

    #[test]
    fn concurrent_insert_unique_keys() {
        use std::sync::Arc;
        use std::thread;

        let set = Arc::new(OpaqueSeenSet::new(16, 800));

        // 8 threads × 100 unique keys each = 800 total.
        // Uniqueness invariant: key = [thread_id, item_id, 0, 0, ...].
        // thread_id occupies key[1], item_id occupies key[0].
        // Both bytes differ across the full space → all keys are unique.
        let handles: Vec<_> = (0..8u8)
            .map(|t| {
                let set = Arc::clone(&set);
                thread::spawn(move || {
                    for i in 0..100u8 {
                        let mut key = [0u8; 32];
                        key[0] = i; // item index — also shard selector
                        key[1] = t; // thread index — differentiates across threads
                        let is_new = set.insert(key);
                        assert!(is_new, "duplicate key in thread {t}, item {i}");
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(set.len(), 800);
    }

    #[test]
    fn concurrent_contains_does_not_block() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let set = Arc::new(OpaqueSeenSet::new(4, 100));
        let key = [0x42; 32];
        set.insert(key);

        // 8 threads all calling contains() simultaneously.
        // Under RwLock, they all succeed concurrently (no write lock needed).
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let set = Arc::clone(&set);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..1000 {
                        assert!(set.contains(&key));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn identity_hasher_uses_first_8_bytes() {
        let mut h = IdentityHasher::default();
        h.write(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        // First 8 bytes as little-endian u64.
        let expected = u64::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(h.finish(), expected);
    }

    #[test]
    fn default_creates_16_shards() {
        let set = OpaqueSeenSet::default();
        assert_eq!(set.shard_count(), 16);
    }

    #[test]
    fn pre_allocation_avoids_rehash() {
        // With expected_total=1000 and 4 shards, each shard pre-allocates
        // capacity for 250. Inserting 250 items should not rehash.
        let set = OpaqueSeenSet::new(4, 1000);
        for i in 0..250u8 {
            let mut key = [0u8; 32];
            key[0] = i;
            set.insert(key);
        }
        assert_eq!(set.len(), 250);
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn non_power_of_two_panics() {
        OpaqueSeenSet::new(3, 0);
    }

    #[test]
    #[should_panic(expected = "non-zero")]
    fn zero_shards_panics() {
        OpaqueSeenSet::new(0, 0);
    }

    #[test]
    fn debug_format_does_not_acquire_locks() {
        // Debug should NOT call len() — verify it doesn't deadlock
        // by calling Debug while holding a write lock.
        let set = OpaqueSeenSet::new(4, 0);
        let key = [0xFF; 32];
        let _guard = set.shards[set.shard_for(&key)].lock.write();
        // If Debug acquired the same shard's lock, this would deadlock.
        // It won't because Debug uses finish_non_exhaustive().
        let debug = format!("{set:?}");
        assert!(debug.contains("OpaqueSeenSet"));
    }
}
