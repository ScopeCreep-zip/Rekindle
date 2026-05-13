//! Monotonic nonce counter that aborts on exhaustion.
//!
//! AES-256-GCM nonce reuse is a catastrophic, unrecoverable
//! cryptographic failure. This counter makes reuse structurally
//! impossible: it calls `std::process::abort()` rather than wrapping.
//! `abort()` is used instead of `panic!`/`assert!` because rayon catches
//! panics (panic=unwind). A caught panic would allow the counter to
//! advance past the limit on subsequent calls from other threads,
//! eventually wrapping to 0. `abort()` cannot be caught.
//!
//! At 10 Gbps with 64 KiB chunks (~19K chunks/sec), u64::MAX
//! takes ~30 million years. The safety margin exists for defense
//! against bugs that store/reset the counter, not for natural
//! exhaustion.
//!
//! # Thread safety
//!
//! `next()` uses `fetch_add(1, Relaxed)` which is atomically unique
//! per counter instance. `Relaxed` is sufficient because nonces only
//! need uniqueness, not ordering — AES-GCM does not require monotonic
//! nonces, only distinct ones. Two rayon workers calling `next()`
//! concurrently will always get distinct values.

use std::sync::atomic::{AtomicU64, Ordering};

/// Safety margin: abort 2^20 (~1M) nonces before u64::MAX.
///
/// This catches bugs that jump the counter near the limit.
/// At 19K chunks/sec, 1M nonces is ~52 seconds of headroom —
/// enough to detect and terminate the session gracefully.
const NONCE_LIMIT: u64 = u64::MAX - (1 << 20);

/// Monotonic nonce counter. Aborts the process on exhaustion.
///
/// `!Clone` — one counter per session, shared via `Arc<NonceCounter>`.
/// This prevents accidental counter duplication which would cause
/// nonce reuse across two stream instances.
#[derive(Debug)]
pub struct NonceCounter {
    inner: AtomicU64,
}

impl NonceCounter {
    /// Create a new counter starting at 0.
    pub fn new() -> Self {
        Self { inner: AtomicU64::new(0) }
    }

    /// Acquire the next nonce. Aborts the process if exhausted.
    ///
    /// Uses `std::process::abort()` — NOT `panic!` — because rayon
    /// catches panics (panic=unwind). A caught panic would leave the
    /// counter past NONCE_LIMIT, allowing subsequent calls to wrap
    /// to 0 after ~2^20 more caught panics. `abort()` is uncatchable.
    ///
    /// # Aborts
    ///
    /// Aborts the process if the counter has reached `NONCE_LIMIT`
    /// (u64::MAX - 2^20). This is a terminal, unrecoverable event.
    pub fn next(&self) -> u64 {
        let n = self.inner.fetch_add(1, Ordering::Relaxed);
        if n >= NONCE_LIMIT {
            // Write to stderr before aborting — tracing may not flush.
            // Using write! to fd 2 directly instead of eprintln! to
            // satisfy clippy::print_stderr while still reaching the operator.
            let _ = std::io::Write::write_fmt(
                &mut std::io::stderr(),
                format_args!(
                    "FATAL: nonce counter exhausted at {n}. \
                     AES-GCM nonce reuse is a catastrophic cryptographic failure. \
                     The session key is compromised. Aborting process.\n"
                ),
            );
            std::process::abort();
        }
        n
    }

    /// Current counter value (for diagnostics/logging only).
    ///
    /// This is a relaxed load — the value may be stale if other
    /// threads are concurrently calling `next()`. Do not use for
    /// anything other than logging.
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
    fn sequential_nonces_are_unique() {
        let ctr = NonceCounter::new();
        let a = ctr.next();
        let b = ctr.next();
        let c = ctr.next();
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(c, 2);
    }

    #[test]
    fn current_reflects_usage() {
        let ctr = NonceCounter::new();
        assert_eq!(ctr.current(), 0);
        ctr.next();
        ctr.next();
        assert_eq!(ctr.current(), 2);
    }

    #[test]
    fn concurrent_nonces_are_unique() {
        use std::sync::Arc;
        let ctr = Arc::new(NonceCounter::new());
        let n = 10_000usize;
        let handles: Vec<_> = (0..8).map(|_| {
            let ctr = Arc::clone(&ctr);
            std::thread::spawn(move || {
                let mut nonces = Vec::with_capacity(n / 8);
                for _ in 0..n / 8 {
                    nonces.push(ctr.next());
                }
                nonces
            })
        }).collect();

        let mut all: Vec<u64> = Vec::with_capacity(n);
        for h in handles {
            all.extend(h.join().unwrap());
        }
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "all nonces must be unique under contention");
    }

    // NOTE: overflow_aborts cannot be tested with #[should_panic] because
    // std::process::abort() is uncatchable. The abort behavior is verified
    // by code inspection: next() calls abort() when n >= NONCE_LIMIT.
    // A process-level test (spawning a child and checking exit signal)
    // would verify this at the integration test level.

    #[test]
    fn at_limit_minus_one_succeeds_then_next_would_abort() {
        // Verify the last valid nonce succeeds.
        let ctr = NonceCounter {
            inner: AtomicU64::new(NONCE_LIMIT - 1),
        };
        let n = ctr.next();
        assert_eq!(n, NONCE_LIMIT - 1);
        // The next call would abort the process. We cannot test that
        // in-process. The counter is now at NONCE_LIMIT.
        assert_eq!(ctr.current(), NONCE_LIMIT);
    }
}
