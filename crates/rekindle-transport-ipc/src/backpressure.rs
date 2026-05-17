//! Global memory guard for bulk transfer backpressure.
//!
//! Hard cap via atomic CAS loop. At 100K+ concurrent agents on the same
//! bus with simultaneous parallel cross-chatter, the optimistic
//! fetch_add+rollback pattern is NOT acceptable — concurrent callers
//! can momentarily exceed the limit by O(callers) × frame_size.
//!
//! This implementation uses `fetch_update` (compare-and-swap loop) which
//! guarantees the limit is NEVER exceeded, even under maximal contention.
//! The CAS loop retries are bounded: each retry means another caller
//! succeeded, so progress is guaranteed (lock-free, not wait-free).
//!
//! `MemoryReservation` is RAII and owns `Arc<GlobalMemoryGuard>` — it is
//! `Send + 'static`, safe to move into rayon closures and through tokio
//! channels. The reservation is released on drop regardless of panic,
//! cancel, or error path.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::error::IpcError;

/// Global memory guard. Hard cap enforced via CAS — never exceeded.
pub struct GlobalMemoryGuard {
    used: AtomicU64,
    limit: u64,
}

impl GlobalMemoryGuard {
    pub fn new(limit: u64) -> Self {
        Self {
            used: AtomicU64::new(0),
            limit,
        }
    }

    /// Try to reserve `bytes`. Returns RAII reservation on success.
    ///
    /// Hard cap: uses compare-and-swap loop. The limit is NEVER exceeded
    /// regardless of concurrency level (100K+ agents). Each CAS retry
    /// means another caller made progress — lock-free guarantee.
    ///
    /// The returned `MemoryReservation` holds `Arc<GlobalMemoryGuard>` and
    /// is `Send + 'static` — safe to move into rayon closures.
    pub fn try_reserve(self: &Arc<Self>, bytes: u64) -> Result<MemoryReservation, IpcError> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                let new = current + bytes;
                if new > self.limit {
                    None
                } else {
                    Some(new)
                }
            })
            .map_err(|current| IpcError::Backpressure { buffered: current })?;

        Ok(MemoryReservation {
            guard: Arc::clone(self),
            bytes,
        })
    }

    /// Current bytes in flight.
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }
}

/// RAII reservation. Releases bytes on drop.
///
/// Owns `Arc<GlobalMemoryGuard>` — `Send + 'static`. Safe to move through
/// rayon closures, tokio channels, and reassembly buffers. Releases on
/// drop regardless of panic, cancel, or error path.
pub struct MemoryReservation {
    guard: Arc<GlobalMemoryGuard>,
    bytes: u64,
}

impl MemoryReservation {
    /// Bytes reserved by this reservation.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        self.guard.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

// MemoryReservation is Send + 'static because it owns Arc<GlobalMemoryGuard>.
// This is required for moving into rayon pool.spawn() closures.
static_assertions::assert_impl_all!(MemoryReservation: Send);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_reserve_release() {
        let guard = Arc::new(GlobalMemoryGuard::new(1000));
        assert_eq!(guard.used(), 0);
        {
            let _r = guard.try_reserve(500).unwrap();
            assert_eq!(guard.used(), 500);
        }
        assert_eq!(guard.used(), 0);
    }

    #[test]
    fn exceeds_limit_rejected_hard() {
        let guard = Arc::new(GlobalMemoryGuard::new(100));
        let _r = guard.try_reserve(80).unwrap();
        assert!(guard.try_reserve(30).is_err());
        assert_eq!(guard.used(), 80);
    }

    #[test]
    fn concurrent_never_exceeds() {
        let guard = Arc::new(GlobalMemoryGuard::new(10_000));
        let handles: Vec<_> = (0..100)
            .map(|_| {
                let g = Arc::clone(&guard);
                std::thread::spawn(move || {
                    let mut reservations = Vec::new();
                    for _ in 0..100 {
                        match g.try_reserve(1) {
                            Ok(r) => {
                                assert!(g.used() <= g.limit());
                                reservations.push(r);
                            }
                            Err(_) => break,
                        }
                    }
                    std::thread::yield_now();
                    drop(reservations);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(guard.used(), 0);
    }

    #[test]
    fn reservation_is_send_and_static() {
        let guard = Arc::new(GlobalMemoryGuard::new(1000));
        let reservation = guard.try_reserve(100).unwrap();
        // Move into a thread — proves Send + 'static
        let handle = std::thread::spawn(move || {
            assert_eq!(reservation.bytes(), 100);
            drop(reservation);
        });
        handle.join().unwrap();
        assert_eq!(guard.used(), 0);
    }
}
