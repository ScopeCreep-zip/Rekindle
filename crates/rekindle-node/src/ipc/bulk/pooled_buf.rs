//! Pool-returnable buffer for the receive-side bulk pipeline.
//!
//! `PooledBuf` wraps a `Vec<u8>` and an `Arc<BufferPool>`. On drop,
//! the buffer is zeroized and returned to the pool via `replenish()`.
//! This closes the receive-side buffer lifecycle into a circle:
//!
//! ```text
//! recv_pool.acquire() → slab
//!   → read_exact(socket, slab)
//!   → dispatcher.dispatch(slab) → PooledBuf
//!   → rayon: open_in_place → DecryptedChunk { plaintext: PooledBuf }
//!   → Reassembler → ReassembledChunk { plaintext: PooledBuf }
//!   → application reads plaintext
//!   → drop PooledBuf → zeroize → recv_pool.replenish(slab)
//! ```
//!
//! After warmup, the receive path touches zero allocator calls.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use super::pool::BufferPool;

/// A buffer that returns itself to a `BufferPool` on drop.
///
/// Implements `Deref<Target=[u8]>` and `DerefMut` for transparent
/// slice access. The `Drop` impl calls `BufferPool::replenish()`
/// which zeroizes the entire capacity via volatile writes before
/// returning the buffer to the free list.
pub struct PooledBuf {
    buf: Vec<u8>,
    pool: Arc<BufferPool>,
}

impl PooledBuf {
    /// Wrap an owned `Vec<u8>` with a pool reference.
    pub fn new(buf: Vec<u8>, pool: Arc<BufferPool>) -> Self {
        Self { buf, pool }
    }

    pub fn len(&self) -> usize { self.buf.len() }
    pub fn is_empty(&self) -> bool { self.buf.is_empty() }
    pub fn truncate(&mut self, len: usize) { self.buf.truncate(len); }

    pub fn drain<R: std::ops::RangeBounds<usize>>(&mut self, range: R) -> std::vec::Drain<'_, u8> {
        self.buf.drain(range)
    }
}

impl Deref for PooledBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] { &self.buf }
}

impl DerefMut for PooledBuf {
    fn deref_mut(&mut self) -> &mut [u8] { &mut self.buf }
}

impl Drop for PooledBuf {
    fn drop(&mut self) {
        let buf = std::mem::take(&mut self.buf);
        self.pool.replenish(buf);
    }
}

// SAFETY: Vec<u8> is Send+Sync. Arc<BufferPool> is Send+Sync.
// No interior mutability beyond what BufferPool provides via Mutex.
static_assertions::assert_impl_all!(PooledBuf: Send, Sync);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooled_buf_returns_to_pool_on_drop() {
        let pool = BufferPool::new();
        let initial = pool.available();
        let slab = pool.acquire();
        assert_eq!(pool.available(), initial - 1);
        let mut pb = PooledBuf::new(slab, Arc::clone(&pool));
        pb.truncate(0);
        drop(pb);
        assert_eq!(pool.available(), initial);
    }

    #[test]
    fn deref_provides_slice_access() {
        let pool = BufferPool::new();
        let mut slab = pool.acquire();
        slab.extend_from_slice(b"hello");
        let pb = PooledBuf::new(slab, pool);
        assert_eq!(&*pb, b"hello");
        assert_eq!(pb.len(), 5);
    }

    #[test]
    fn deref_mut_allows_in_place_modification() {
        let pool = BufferPool::new();
        let mut slab = pool.acquire();
        slab.extend_from_slice(&[0xAB; 10]);
        let mut pb = PooledBuf::new(slab, pool);
        pb[0] = 0xCD;
        assert_eq!(pb[0], 0xCD);
    }

    #[test]
    fn drain_removes_prefix() {
        let pool = BufferPool::new();
        let mut slab = pool.acquire();
        slab.extend_from_slice(b"HEADERpayload");
        let mut pb = PooledBuf::new(slab, pool);
        pb.drain(..6);
        assert_eq!(&*pb, b"payload");
    }

    #[test]
    fn truncate_shortens() {
        let pool = BufferPool::new();
        let mut slab = pool.acquire();
        slab.extend_from_slice(&[0xAB; 100]);
        let mut pb = PooledBuf::new(slab, pool);
        pb.truncate(10);
        assert_eq!(pb.len(), 10);
    }

    #[test]
    fn empty_buf_returns_to_pool() {
        let pool = BufferPool::new();
        let initial = pool.available();
        let pb = PooledBuf::new(Vec::new(), Arc::clone(&pool));
        drop(pb);
        // Empty vec has capacity < SLAB_SIZE, so replenish drops it.
        // Pool count unchanged — this is correct behavior (undersized
        // slabs are rejected by replenish to prevent pool pollution).
        assert_eq!(pool.available(), initial);
    }
}
