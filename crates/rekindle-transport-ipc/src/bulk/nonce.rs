//! Monotonic nonce counter that aborts on exhaustion.
//!
//! AES-256-GCM nonce reuse is catastrophic. This counter makes reuse
//! structurally impossible: `abort()` instead of wrapping.
//!
//! Uses `abort()` not `panic!()` because rayon catches panics. A caught
//! panic would allow the counter to advance past the limit.
//!
//! At 10 Gbps with 64 KiB chunks (~19K/sec), u64::MAX takes ~30M years.
//! The safety margin catches bugs that store/reset the counter.

use std::sync::atomic::{AtomicU64, Ordering};

/// Abort 2^20 (~1M) nonces before u64::MAX.
const NONCE_LIMIT: u64 = u64::MAX - (1 << 20);

/// Monotonic nonce counter. Aborts on exhaustion.
///
/// `!Clone` — one counter per session, shared via `Arc<NonceCounter>`.
/// Prevents accidental duplication which would cause nonce reuse.
#[derive(Debug)]
pub struct NonceCounter {
    inner: AtomicU64,
}

impl NonceCounter {
    pub fn new() -> Self {
        Self {
            inner: AtomicU64::new(0),
        }
    }

    /// Acquire the next nonce. Aborts the process if exhausted.
    pub fn next(&self) -> u64 {
        let n = self.inner.fetch_add(1, Ordering::Relaxed);
        if n >= NONCE_LIMIT {
            let _ = std::io::Write::write_fmt(
                &mut std::io::stderr(),
                format_args!(
                    "FATAL: nonce counter exhausted at {n}. \
                     AES-GCM nonce reuse is catastrophic. Aborting.\n"
                ),
            );
            std::process::abort();
        }
        n
    }

    /// Current value (diagnostics only — may be stale under contention).
    pub fn current(&self) -> u64 {
        self.inner.load(Ordering::Relaxed)
    }
}

impl Default for NonceCounter {
    fn default() -> Self {
        Self::new()
    }
}

static_assertions::assert_impl_all!(NonceCounter: Send, Sync);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_unique() {
        let ctr = NonceCounter::new();
        assert_eq!(ctr.next(), 0);
        assert_eq!(ctr.next(), 1);
        assert_eq!(ctr.next(), 2);
    }

    #[test]
    fn concurrent_unique() {
        use std::sync::Arc;
        let ctr = Arc::new(NonceCounter::new());
        let n = 10_000usize;
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let c = Arc::clone(&ctr);
                std::thread::spawn(move || {
                    (0..n / 8).map(|_| c.next()).collect::<Vec<_>>()
                })
            })
            .collect();
        let mut all: Vec<u64> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n);
    }
}
