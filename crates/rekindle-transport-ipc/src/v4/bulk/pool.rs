//! mmap-backed buffer pool on `SlabPool<MmapSlab, ImmediateReturn, MmapSlabLifecycle>`.
//!
//! The buffer never crosses a thread. The `SlabGuard` is `!Send` by
//! construction and stays on the ring thread. Cross-thread payloads are
//! `Vec<u8>` bytes, not pooled buffers. The byte→buffer copy happens on
//! the ring thread immediately before io_uring submission.
//!
//! `ImmediateReturn` is the correct gate because AF_UNIX copies on send
//! (`IORING_OP_SEND_ZC` silently falls back to copying on AF_UNIX). The
//! buffer is reusable the instant the send CQE returns. When a non-AF_UNIX
//! transport is added, `ImmediateReturn` is swapped to `IoUringNotifGate`
//! — a generic-parameter change, not a re-architecture.
//!
//! Slabs are backed by `mmap(MAP_ANONYMOUS | MAP_PRIVATE | MAP_POPULATE)`
//! with `MADV_DONTDUMP` and `MADV_DONTFORK`. Volatile zeroize on reset.
//! Stable base addresses enable `io_uring register_buffers`.

use std::sync::Arc;

use rekindle_transport_buff::traits::{ImmediateReturn, SlotLifecycle};
use rekindle_transport_buff::SlabPool;

use crate::v4::wire::constants::{AEAD_TAG_LEN, ENVELOPE_LEN, STREAM_HEADER_LEN};

// ---------------------------------------------------------------------------
// MmapRegion — platform-specific mmap backing
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[allow(unsafe_code)]
mod mmap_region {
    use std::ptr;

    pub struct MmapRegion {
        ptr: *mut u8,
        capacity: usize,
    }

    // SAFETY: MmapRegion is process-private (MAP_PRIVATE), exclusively
    // owned by one slab at a time. The raw pointer is stable (mmap regions
    // never move) and the memory is not aliased.
    unsafe impl Send for MmapRegion {}
    unsafe impl Sync for MmapRegion {}

    impl MmapRegion {
        pub fn new(capacity: usize) -> Self {
            assert!(capacity > 0, "mmap region capacity must be > 0");
            // SAFETY: mmap with MAP_ANONYMOUS | MAP_PRIVATE creates a
            // process-private zero-filled region. MAP_POPULATE pre-faults.
            let ptr = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    capacity,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_ANONYMOUS | libc::MAP_PRIVATE | libc::MAP_POPULATE,
                    -1,
                    0,
                )
            };
            assert_ne!(
                ptr,
                libc::MAP_FAILED,
                "mmap failed: {}",
                std::io::Error::last_os_error()
            );
            let ptr = ptr.cast::<u8>();

            // SAFETY: ptr is a valid mmap'd region of `capacity` bytes.
            unsafe {
                libc::madvise(ptr.cast(), capacity, libc::MADV_DONTDUMP);
            }
            // SAFETY: same valid mmap'd region.
            unsafe {
                libc::madvise(ptr.cast(), capacity, libc::MADV_DONTFORK);
            }

            Self { ptr, capacity }
        }

        pub fn as_mut_slice(&mut self) -> &mut [u8] {
            // SAFETY: ptr is valid for capacity bytes, exclusively owned.
            unsafe { std::slice::from_raw_parts_mut(self.ptr, self.capacity) }
        }

        pub fn as_slice(&self) -> &[u8] {
            // SAFETY: ptr is valid for capacity bytes.
            unsafe { std::slice::from_raw_parts(self.ptr, self.capacity) }
        }

        pub fn capacity(&self) -> usize {
            self.capacity
        }

        pub fn base_ptr(&self) -> *const u8 {
            self.ptr
        }
    }

    impl Drop for MmapRegion {
        fn drop(&mut self) {
            // SAFETY: ptr and capacity are from a successful mmap call.
            unsafe {
                libc::munmap(self.ptr.cast(), self.capacity);
            }
        }
    }
}

#[cfg(not(unix))]
mod mmap_region {
    pub struct MmapRegion {
        buf: Vec<u8>,
    }

    impl MmapRegion {
        pub fn new(capacity: usize) -> Self {
            Self {
                buf: vec![0u8; capacity],
            }
        }
        pub fn as_mut_slice(&mut self) -> &mut [u8] {
            &mut self.buf
        }
        pub fn as_slice(&self) -> &[u8] {
            &self.buf
        }
        pub fn capacity(&self) -> usize {
            self.buf.len()
        }
        pub fn base_ptr(&self) -> *const u8 {
            self.buf.as_ptr()
        }
    }
}

// ---------------------------------------------------------------------------
// MmapSlab — the payload type stored in SlabPool
// ---------------------------------------------------------------------------

/// Default slab capacity: one maximum Data lane frame.
/// 32 (Envelope) + 32 (Header) + 16 MiB (ciphertext) + 16 (tag).
pub const DEFAULT_SLAB_CAPACITY: usize =
    ENVELOPE_LEN + STREAM_HEADER_LEN + (16 * 1024 * 1024) + AEAD_TAG_LEN;

/// A fixed-size mmap-backed memory region with a logical length.
/// Stable base address. Anonymous pages pass `sendpage_ok()`.
pub struct MmapSlab {
    region: mmap_region::MmapRegion,
    len: usize,
}

impl MmapSlab {
    pub fn capacity(&self) -> usize {
        self.region.capacity()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.region.as_slice()[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        let len = self.len;
        &mut self.region.as_mut_slice()[..len]
    }

    /// Full-capacity mutable access for in-place fill.
    /// Caller must call `set_len` after writing.
    pub fn buf_mut(&mut self) -> &mut [u8] {
        self.region.as_mut_slice()
    }

    pub fn set_len(&mut self, new_len: usize) {
        assert!(new_len <= self.capacity());
        self.len = new_len;
    }

    pub fn base_ptr(&self) -> *const u8 {
        self.region.base_ptr()
    }

    fn zeroize_and_reset(&mut self) {
        if self.len > 0 {
            zeroize::Zeroize::zeroize(&mut self.region.as_mut_slice()[..self.len]);
        }
        self.len = 0;
    }
}

// ---------------------------------------------------------------------------
// MmapSlabLifecycle — SlotLifecycle impl for SlabPool
// ---------------------------------------------------------------------------

/// Constructs mmap-backed slabs and volatile-zeroizes on reuse.
pub struct MmapSlabLifecycle {
    capacity: usize,
}

impl MmapSlabLifecycle {
    pub fn new(capacity: usize) -> Self {
        Self { capacity }
    }
}

impl SlotLifecycle<MmapSlab> for MmapSlabLifecycle {
    fn construct(&self) -> MmapSlab {
        MmapSlab {
            region: mmap_region::MmapRegion::new(self.capacity),
            len: 0,
        }
    }

    fn reset(&self, slab: &mut MmapSlab) {
        slab.zeroize_and_reset();
    }
}

// ---------------------------------------------------------------------------
// BufferPool — the public API wrapping SlabPool
// ---------------------------------------------------------------------------

/// Type alias for the concrete SlabPool used by the IPC transport.
pub type IpcSlabPool = SlabPool<MmapSlab, ImmediateReturn, MmapSlabLifecycle>;

/// Pre-allocated mmap-backed buffer pool. Thread-safe. Bounded.
/// Zero-allocation after warmup.
///
/// One global instance per process, shared across all connections via `Arc`.
/// The `SlabGuard` returned by `try_acquire` is `!Send` — the buffer stays
/// on the acquiring thread (the ring thread).
pub struct BufferPool {
    inner: IpcSlabPool,
    slab_capacity: usize,
}

impl BufferPool {
    /// Create a send-side pool with `slab_count` slabs at the default
    /// slab capacity (16 MiB + overhead).
    pub fn new(slab_count: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: SlabPool::new(
                slab_count,
                ImmediateReturn,
                MmapSlabLifecycle::new(DEFAULT_SLAB_CAPACITY),
            ),
            slab_capacity: DEFAULT_SLAB_CAPACITY,
        })
    }

    /// Create a recv-side pool with plaintext-sized slabs.
    pub fn for_recv(slab_count: usize) -> Arc<Self> {
        let plaintext_capacity = (crate::v4::wire::constants::MAX_BODY_LEN_DATA as usize)
            - STREAM_HEADER_LEN
            - AEAD_TAG_LEN;
        Arc::new(Self {
            inner: SlabPool::new(
                slab_count,
                ImmediateReturn,
                MmapSlabLifecycle::new(plaintext_capacity),
            ),
            slab_capacity: plaintext_capacity,
        })
    }

    /// Create a pool with specific slab capacity.
    pub fn with_capacity(slab_count: usize, slab_capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: SlabPool::new(
                slab_count,
                ImmediateReturn,
                MmapSlabLifecycle::new(slab_capacity),
            ),
            slab_capacity,
        })
    }

    /// Access the underlying SlabPool directly.
    pub fn pool(&self) -> &IpcSlabPool {
        &self.inner
    }

    /// Acquire a slab. Returns None if the pool is exhausted.
    /// The returned `SlabGuard` is `!Send` — stays on the acquiring thread.
    pub fn try_acquire(&self) -> Option<rekindle_transport_buff::SlabGuard<'_, MmapSlab, ImmediateReturn, MmapSlabLifecycle>> {
        self.inner.try_acquire()
    }

    /// Release a slab back through the gate.
    pub fn release(&self, guard: rekindle_transport_buff::SlabGuard<'_, MmapSlab, ImmediateReturn, MmapSlabLifecycle>) {
        self.inner.release(guard);
    }

    /// Reclaim gate-cleared slabs back to the free list.
    pub fn reclaim(&self) -> usize {
        self.inner.reclaim()
    }

    /// Number of slabs currently available in the pool.
    pub fn available(&self) -> usize {
        self.inner.available()
    }

    /// Total slab count.
    pub fn total(&self) -> usize {
        self.inner.capacity()
    }

    /// The slab capacity this pool was constructed with.
    pub fn slab_capacity(&self) -> usize {
        self.slab_capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_starts_full() {
        let pool = BufferPool::with_capacity(8, 1024);
        assert_eq!(pool.available(), 8);
    }

    #[test]
    fn acquire_and_release() {
        let pool = BufferPool::with_capacity(4, 1024);
        let guard = pool.try_acquire().unwrap();
        assert_eq!(pool.available(), 3);
        pool.release(guard);
        pool.reclaim();
        assert_eq!(pool.available(), 4);
    }

    #[test]
    fn try_acquire_returns_none_when_empty() {
        let pool = BufferPool::with_capacity(1, 1024);
        let _held = pool.try_acquire().unwrap();
        assert_eq!(pool.available(), 0);
        assert!(pool.try_acquire().is_none());
    }

    #[test]
    fn slab_zeroized_on_release() {
        let pool = BufferPool::with_capacity(1, 256);
        {
            let mut guard = pool.try_acquire().unwrap();
            guard.buf_mut()[..128].copy_from_slice(&[0xFF; 128]);
            guard.set_len(128);
            pool.release(guard);
            pool.reclaim();
        }
        let mut guard = pool.try_acquire().unwrap();
        assert_eq!(guard.len(), 0);
        assert!(
            guard.buf_mut()[..128].iter().all(|&b| b == 0),
            "slab must be fully zeroized after return to pool"
        );
    }

    #[test]
    fn deref_access() {
        let pool = BufferPool::with_capacity(1, 1024);
        let mut guard = pool.try_acquire().unwrap();
        guard.buf_mut()[..5].copy_from_slice(b"hello");
        guard.set_len(5);
        assert_eq!(guard.as_slice(), b"hello");
    }
}
