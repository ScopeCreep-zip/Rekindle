//! Secret bytes in locked memory (plan E7.1, security roadmap 5.2).
//!
//! A [`LockedBox`] holds `N` secret bytes on the heap in pages it locks for
//! its lifetime, so the OS does not write them to swap (or, on Windows, the
//! page file). It zeroizes them on drop and only then unlocks.
//!
//! Locking goes through the `region` crate's `lock`, a safe function
//! (`mlock` on Unix, `VirtualLock` on Windows); the crate stays
//! `#![forbid(unsafe_code)]`. rustix's `mlock` is `unsafe` and `mlockall`
//! would lock Veilid's whole heap, so neither is used.
//!
//! Locks do not nest: unlocking a page unlocks it for every value on it.
//! So each box owns its pages outright: the buffer is aligned to and padded
//! out to [`PAGE_ALIGN`], the largest page size Rekindle runs on (16 KiB,
//! Apple silicon; 4 KiB elsewhere divides it), and no other value shares
//! them.
//!
//! The OS lock budget (`RLIMIT_MEMLOCK`) may be small. A refused lock is
//! logged and the secret still loads: a refusal lowers no other protection
//! (core dumps stay off, the bytes are still zeroized), and refusing to
//! unlock the identity would protect nothing.

use zeroize::{Zeroize, Zeroizing};

/// The alignment and minimum size of a box's buffer: 16 KiB.
pub const PAGE_ALIGN: usize = 16 * 1024;

/// The buffer: alone on its pages (see the module docs).
#[repr(C, align(16384))]
struct Pages<const N: usize>(Zeroizing<[u8; N]>);

/// `N` secret bytes in locked, zeroize-on-drop memory.
pub struct LockedBox<const N: usize> {
    pages: Box<Pages<N>>,
    guard: Option<region::LockGuard>,
}

impl<const N: usize> LockedBox<N> {
    /// Allocate and lock a zeroed buffer, then let `init` write the secret
    /// straight into it, so the secret is never on the stack or in an
    /// unlocked heap block first.
    pub fn new_with(init: impl FnOnce(&mut [u8; N])) -> Self {
        let mut pages = Box::new(Pages(Zeroizing::new([0u8; N])));
        let guard = match region::lock(pages.0.as_ptr(), N.max(1)) {
            Ok(guard) => Some(guard),
            Err(e) => {
                tracing::warn!(error = %e, bytes = N, "secret memory not locked");
                None
            }
        };
        init(&mut pages.0);
        Self { pages, guard }
    }

    /// Copy `bytes` into a new box. The caller zeroizes its own copy.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; N]) -> Self {
        Self::new_with(|buf| buf.copy_from_slice(bytes))
    }

    /// The secret bytes.
    #[must_use]
    pub fn expose(&self) -> &[u8; N] {
        &self.pages.0
    }

    /// Whether the OS locked the pages (false when the lock budget refused).
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.guard.is_some()
    }
}

impl<const N: usize> Drop for LockedBox<N> {
    fn drop(&mut self) {
        // Zeroize while still locked, then unlock; the box frees after.
        self.pages.0.zeroize();
        self.guard.take();
    }
}

/// Dropping a box zeroizes its bytes (the `Drop` above).
impl<const N: usize> zeroize::ZeroizeOnDrop for LockedBox<N> {}

impl<const N: usize> std::fmt::Debug for LockedBox<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockedBox")
            .field("len", &N)
            .field("locked", &self.is_locked())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "locked_tests.rs"]
mod tests;
