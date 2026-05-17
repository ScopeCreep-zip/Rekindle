#![allow(unsafe_code)]
//! Pre-allocated buffer pool for zero-allocation bulk encryption.
//!
//! Each slab: capacity for one max-size bulk frame (~65.5 KiB).
//! Lifecycle: acquire -> rayon encrypt -> channel send -> socket write -> replenish.
//!
//! replenish() zeroizes entire capacity via volatile writes (zeroize crate)
//! before returning to free list. Cannot be elided by optimizer.
//!
//! After warmup (all slabs cycled once), the pipeline touches zero
//! allocator calls.

use std::sync::Arc;
use parking_lot::Mutex;

use super::frame::{HEADER_LEN, MAX_CHUNK_PLAIN, TAG_LEN};

/// Size of each buffer slab: header + max plaintext + tag.
pub const SLAB_SIZE: usize = HEADER_LEN + MAX_CHUNK_PLAIN + TAG_LEN;

/// Pre-allocated buffer pool for bulk encryption/decryption.
///
/// Thread-safe via `parking_lot::Mutex` + `parking_lot::Condvar`.
/// When the pool is empty, `acquire()` parks the calling OS thread
/// on the condvar (zero CPU). `replenish()` wakes one parked waiter.
/// This replaces the previous spin loop which burned 100% CPU and
/// caused system lockup when the write task stalled.
pub struct BufferPool {
    free: Mutex<Vec<Vec<u8>>>,
    not_empty: parking_lot::Condvar,
    slab_count: usize,
}

impl BufferPool {
    /// Create a pool with `slab_count` pre-allocated, pre-faulted slabs.
    pub fn new(slab_count: usize) -> Arc<Self> {
        let mut free = Vec::with_capacity(slab_count);
        for _ in 0..slab_count {
            // Pre-fault all pages by writing zeros.
            let mut v = vec![0u8; SLAB_SIZE];
            v.clear();
            free.push(v);
        }
        Arc::new(Self {
            free: Mutex::new(free),
            not_empty: parking_lot::Condvar::new(),
            slab_count,
        })
    }

    /// Try to acquire a slab. Returns None if pool exhausted.
    pub fn try_acquire(&self) -> Option<Vec<u8>> {
        self.free.lock().pop()
    }

    /// Acquire a slab, parking on condvar if pool is empty.
    /// Zero CPU when waiting. Woken instantly by `replenish()`.
    pub fn acquire(&self) -> Vec<u8> {
        let mut free = self.free.lock();
        loop {
            if let Some(buf) = free.pop() {
                return buf;
            }
            // Park the OS thread on the condvar. Zero CPU consumed.
            // The rayon worker sleeps here — the OS scheduler can run
            // the write task (which calls replenish) on this core.
            // Woken by replenish() calling notify_one().
            self.not_empty.wait(&mut free);
        }
    }

    /// Return a slab after zeroizing its entire capacity.
    ///
    /// Zeroization via volatile writes prevents plaintext remnants from
    /// persisting in freed slabs. Undersized slabs (from unexpected
    /// reallocation) are silently dropped to prevent pool pollution.
    ///
    /// Signals one parked `acquire()` caller via condvar after push.
    pub fn replenish(&self, mut buf: Vec<u8>) {
        let cap = buf.capacity();
        if cap > 0 {
            // SAFETY: Vec allocation is valid and contiguous for `capacity`
            // bytes. We construct a slice over the full capacity to zero
            // bytes beyond len that may contain plaintext from truncate().
            let full_slice = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr(), cap) };
            zeroize::Zeroize::zeroize(full_slice);
        }
        buf.clear();
        if buf.capacity() >= SLAB_SIZE {
            let mut free = self.free.lock();
            // Defense-in-depth: never exceed slab_count. Prevents unbounded
            // growth if a buffer that was NOT acquired from this pool is
            // accidentally replenished. The buffer is already zeroized above;
            // excess slabs are simply deallocated by Vec::drop.
            if free.len() < self.slab_count {
                free.push(buf);
                // Notify under the lock — matches rayon's wake_specific_thread pattern.
                // Wakes exactly one parked acquire() caller.
                self.not_empty.notify_one();
            }
        }
    }

    /// Current number of free slabs.
    pub fn available(&self) -> usize {
        self.free.lock().len()
    }

    /// Total slabs allocated at construction.
    pub fn total(&self) -> usize {
        self.slab_count
    }
}

static_assertions::assert_impl_all!(BufferPool: Send, Sync);

/// RAII buffer that returns itself to the pool on drop.
/// Zeroized via `BufferPool::replenish()`.
pub struct PooledBuf {
    buf: Vec<u8>,
    pool: Arc<BufferPool>,
}

impl PooledBuf {
    pub fn new(buf: Vec<u8>, pool: Arc<BufferPool>) -> Self {
        Self { buf, pool }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn truncate(&mut self, len: usize) {
        self.buf.truncate(len);
    }

    pub fn drain<R: std::ops::RangeBounds<usize>>(
        &mut self,
        range: R,
    ) -> std::vec::Drain<'_, u8> {
        self.buf.drain(range)
    }
}

impl std::ops::Deref for PooledBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.buf
    }
}

impl std::ops::DerefMut for PooledBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

impl Drop for PooledBuf {
    fn drop(&mut self) {
        let buf = std::mem::take(&mut self.buf);
        self.pool.replenish(buf);
    }
}

static_assertions::assert_impl_all!(PooledBuf: Send, Sync);

/// Heap-allocated buffer that zeroizes full capacity on drop.
///
/// No pool reference. No mutex in Drop. No reallocation. Lock-free.
///
/// Used exclusively on the recv path where the frame allocation is
/// unavoidable (`read_exact` requires a buffer sized to the length
/// prefix, which is unknown until read). The send path uses `PooledBuf`
/// (pool-backed, closed-loop acquire/replenish).
///
/// Drop zeroizes `capacity()` bytes (not `len()`) via volatile writes,
/// matching the kernel's `kfree_sensitive(ksize(mem))` pattern. This
/// ensures plaintext remnants from `truncate()` are wiped.
///
/// Drop is infallible — never panics, never takes a lock.
#[derive(Debug)]
pub struct ZeroizingBuf(Vec<u8>);

impl ZeroizingBuf {
    #[inline]
    pub fn new(buf: Vec<u8>) -> Self {
        Self(buf)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.clone()
    }
}

impl std::ops::Deref for ZeroizingBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl std::ops::DerefMut for ZeroizingBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl Drop for ZeroizingBuf {
    fn drop(&mut self) {
        let cap = self.0.capacity();
        if cap > 0 {
            // SAFETY: Vec allocation is contiguous for capacity() bytes.
            // Zeroize full capacity — plaintext may exist beyond len()
            // after truncate(). Volatile writes cannot be elided.
            let full = unsafe { std::slice::from_raw_parts_mut(self.0.as_mut_ptr(), cap) };
            zeroize::Zeroize::zeroize(full);
        }
        // Vec::drop fires implicitly — returns memory to OS allocator.
    }
}

static_assertions::assert_impl_all!(ZeroizingBuf: Send, Sync);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_starts_full() {
        let pool = BufferPool::new(64);
        assert_eq!(pool.available(), 64);
    }

    #[test]
    fn acquire_replenish_cycle() {
        let pool = BufferPool::new(64);
        let buf = pool.acquire();
        assert_eq!(pool.available(), 63);
        pool.replenish(buf);
        assert_eq!(pool.available(), 64);
    }

    #[test]
    fn pooled_buf_returns_on_drop() {
        let pool = BufferPool::new(16);
        let initial = pool.available();
        let slab = pool.acquire();
        let pb = PooledBuf::new(slab, Arc::clone(&pool));
        drop(pb);
        assert_eq!(pool.available(), initial);
    }
}
