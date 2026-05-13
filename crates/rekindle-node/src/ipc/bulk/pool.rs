//! Pre-allocated buffer pool for zero-allocation bulk encryption.
//!
//! Each slab is a `Vec<u8>` with capacity for one maximum-size bulk
//! frame (header + max plaintext + tag = 65,549 bytes). Slabs are
//! acquired by rayon workers for encryption, sent through a crossbeam
//! channel to the writer task, and returned to the pool after the
//! socket write completes.
//!
//! # Lifecycle
//!
//! ```text
//! Pool::acquire() → Vec<u8> (capacity=SLAB_SIZE, len=0)
//!   → rayon worker writes header + plaintext + encrypts in-place + appends tag
//!   → crossbeam::Sender::send(Vec<u8>)
//!   → writer task receives Vec<u8>
//!   → writer task calls writev/write_all
//!   → writer task calls Pool::replenish(Vec<u8>)
//!   → Vec<u8> is cleared (len=0) and pushed back to the free list
//! ```
//!
//! # Zero-allocation invariant
//!
//! After warmup (all 256 slabs have been acquired and returned at
//! least once), the encrypt→write→return cycle touches zero allocator
//! calls. The `Vec<u8>` capacity is preserved across clear() calls.
//!
//! # Sizing
//!
//! 256 slabs × 65,549 bytes = ~16.4 MiB. At 10 Gbps with 64 KiB
//! chunks (~19,073 chunks/sec) and 4 rayon workers, steady-state
//! usage is ~4 slabs. The 256-slab pool provides 64× headroom.

use std::sync::Arc;
use parking_lot::Mutex;

use super::frame::{HEADER_LEN, MAX_CHUNK_PLAIN, TAG_LEN};

/// Size of each buffer slab: header + max plaintext + tag.
pub const SLAB_SIZE: usize = HEADER_LEN + MAX_CHUNK_PLAIN + TAG_LEN;

/// Number of pre-allocated slabs.
const SLAB_COUNT: usize = 256;

/// Pre-allocated buffer pool for bulk encryption/decryption.
///
/// Thread-safe via `parking_lot::Mutex`. At the target operation rate
/// (~19K ops/sec), the mutex is uncontended because acquire and
/// return rates are balanced.
///
/// If profiling shows >1% time in `parking_lot_core::park` on this
/// mutex, replace the `Mutex<Vec<Vec<u8>>>` with a
/// `crossbeam::queue::ArrayQueue<Vec<u8>>` (lock-free, MPMC).
pub struct BufferPool {
    free: Mutex<Vec<Vec<u8>>>,
}

impl BufferPool {
    /// Create a new pool with `SLAB_COUNT` pre-allocated slabs.
    ///
    /// Each slab is pre-faulted: all pages are touched at construction
    /// time so page faults do not occur during the hot path.
    pub fn new() -> Arc<Self> {
        let mut free = Vec::with_capacity(SLAB_COUNT);
        for _ in 0..SLAB_COUNT {
            // Pre-fault all pages by writing zeros. This ensures page
            // faults happen at startup, not on the hot encryption path.
            let mut v = vec![0u8; SLAB_SIZE];
            v.clear(); // Reset len to 0, preserving capacity + faulted pages.
            free.push(v);
        }
        Arc::new(Self {
            free: Mutex::new(free),
        })
    }

    /// Acquire a slab from the pool.
    ///
    /// Returns `None` if the pool is exhausted. The caller should
    /// either spin-retry or fall back to a fresh allocation.
    ///
    /// The returned `Vec<u8>` has `len() == 0` and
    /// `capacity() >= SLAB_SIZE`.
    pub fn try_acquire(&self) -> Option<Vec<u8>> {
        self.free.lock().pop()
    }

    /// Acquire a slab, spinning until one is available.
    ///
    /// In steady state this never spins because the pool has 64×
    /// headroom over the in-flight slab count. Spinning only occurs
    /// under extreme burst load where all 256 slabs are simultaneously
    /// in the encrypt→write pipeline.
    pub fn acquire(&self) -> Vec<u8> {
        loop {
            if let Some(buf) = self.try_acquire() {
                return buf;
            }
            std::thread::yield_now();
        }
    }

    /// Return a slab to the pool.
    ///
    /// The slab's length is cleared to 0; capacity is preserved.
    /// Slabs whose capacity has changed (e.g., due to an unexpected
    /// `extend` that triggered reallocation) are silently dropped
    /// to prevent pool pollution.
    pub fn replenish(&self, mut buf: Vec<u8>) {
        buf.clear();
        if buf.capacity() >= SLAB_SIZE {
            self.free.lock().push(buf);
        }
        // else: capacity shrunk somehow — drop it. The pool will
        // self-heal as other slabs are returned.
    }

    /// Current number of free slabs. For monitoring and diagnostics.
    pub fn available(&self) -> usize {
        self.free.lock().len()
    }

    /// Pool metrics snapshot for observability endpoints.
    pub fn metrics(&self) -> PoolMetrics {
        let available = self.free.lock().len();
        PoolMetrics {
            total: SLAB_COUNT,
            available,
            in_flight: SLAB_COUNT.saturating_sub(available),
            slab_size: SLAB_SIZE,
        }
    }
}

/// Buffer pool metrics snapshot.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct PoolMetrics {
    /// Total slabs allocated at pool creation.
    pub total: usize,
    /// Slabs currently free in the pool.
    pub available: usize,
    /// Slabs currently in the encrypt→write pipeline.
    pub in_flight: usize,
    /// Size of each slab in bytes.
    pub slab_size: usize,
}

static_assertions::assert_impl_all!(BufferPool: Send, Sync);

// No Default impl — BufferPool::new() returns Arc<Self> with
// pre-allocated slabs. A Default that creates an empty pool
// would be a footgun: every acquire() would spin forever.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_starts_full() {
        let pool = BufferPool::new();
        assert_eq!(pool.available(), SLAB_COUNT);
    }

    #[test]
    fn acquire_returns_empty_slab_with_capacity() {
        let pool = BufferPool::new();
        let buf = pool.acquire();
        assert_eq!(buf.len(), 0);
        assert!(buf.capacity() >= SLAB_SIZE);
        assert_eq!(pool.available(), SLAB_COUNT - 1);
    }

    #[test]
    fn replenish_returns_slab_to_pool() {
        let pool = BufferPool::new();
        let buf = pool.acquire();
        assert_eq!(pool.available(), SLAB_COUNT - 1);
        pool.replenish(buf);
        assert_eq!(pool.available(), SLAB_COUNT);
    }

    #[test]
    fn replenish_clears_length() {
        let pool = BufferPool::new();
        let mut buf = pool.acquire();
        buf.extend_from_slice(&[0xAA; 100]);
        assert_eq!(buf.len(), 100);
        pool.replenish(buf);

        // Re-acquire and verify length is 0.
        let buf2 = pool.acquire();
        assert_eq!(buf2.len(), 0);
        pool.replenish(buf2);
    }

    #[test]
    fn exhaustion_returns_none() {
        let pool = BufferPool::new();
        let mut held = Vec::new();
        for _ in 0..SLAB_COUNT {
            held.push(pool.try_acquire().expect("should not exhaust yet"));
        }
        assert!(pool.try_acquire().is_none());
        assert_eq!(pool.available(), 0);

        for buf in held {
            pool.replenish(buf);
        }
        assert_eq!(pool.available(), SLAB_COUNT);
    }

    #[test]
    fn no_drain_over_many_cycles() {
        let pool = BufferPool::new();
        for _ in 0..10_000 {
            let buf = pool.acquire();
            pool.replenish(buf);
        }
        assert_eq!(pool.available(), SLAB_COUNT);
    }

    #[test]
    fn undersized_slab_dropped_on_replenish() {
        let pool = BufferPool::new();
        // Replenish a tiny vec — should be dropped, not accepted.
        let tiny = Vec::with_capacity(10);
        pool.replenish(tiny);
        // Pool count unchanged from initial.
        assert_eq!(pool.available(), SLAB_COUNT);
    }
}
