//! Loom model-checking tests for [`CreditGuard`].
//!
//! Run with `RUSTFLAGS="--cfg loom" cargo test --test loom_credit`.
//!
//! # What these models prove
//!
//! 1. The CAS loop in `try_reserve` is linearizable: two concurrent
//!    reservers each reserving 1 byte against a ceiling of 1 see exactly
//!    one success and one failure in every interleaving.
//!
//! 2. A `release` after a successful `try_reserve` makes the freed
//!    credit visible to the next `try_reserve` (the Release/Acquire
//!    ordering on the counter).
//!
//! 3. No lost updates: the final `inflight` value after concurrent
//!    reserve+release cycles is always consistent.

#![cfg(loom)]

use loom::sync::Arc;
use loom::thread;

use rekindle_transport_buff::CreditGuard;

/// Model 1: Two threads each try to reserve 1 byte against a ceiling of 1.
/// Exactly one succeeds. Proves the CAS loop is linearizable.
#[test]
fn two_reservers_ceiling_one() {
    loom::model(|| {
        let guard = Arc::new(CreditGuard::new(1));

        let g1 = guard.clone();
        let t1 = thread::spawn(move || g1.try_reserve(1));

        let g2 = guard.clone();
        let t2 = thread::spawn(move || g2.try_reserve(1));

        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();

        // Exactly one must succeed.
        let successes = usize::from(r1) + usize::from(r2);
        assert_eq!(successes, 1, "exactly one of two reservers must win");
        assert_eq!(guard.inflight(), 1);
    });
}

/// Model 2: Thread A reserves, thread B reserves (fails), thread A
/// releases, thread B retries and succeeds. Proves the Release ordering
/// on `release` makes the freed credit visible to the next `try_reserve`'s
/// Acquire load.
#[test]
fn release_makes_credit_visible() {
    loom::model(|| {
        let guard = Arc::new(CreditGuard::new(1));

        // Thread A reserves successfully.
        assert!(guard.try_reserve(1));

        let g = guard.clone();
        let consumer = thread::spawn(move || {
            // Spin until reserve succeeds (thread A will release).
            loop {
                if g.try_reserve(1) {
                    return;
                }
                loom::thread::yield_now();
            }
        });

        // Release — makes the credit visible to the consumer's retry.
        guard.release(1);

        consumer.join().unwrap();

        // Consumer reserved 1, so inflight == 1.
        assert_eq!(guard.inflight(), 1);
    });
}

/// Model 3: Two threads each do a reserve(1) + release(1) cycle against
/// a ceiling of 2. After both join, inflight must be 0. Proves no lost
/// updates in the CAS loop under concurrent reserve/release interleaving.
#[test]
fn concurrent_reserve_release_no_lost_updates() {
    loom::model(|| {
        let guard = Arc::new(CreditGuard::new(2));

        let g1 = guard.clone();
        let t1 = thread::spawn(move || {
            assert!(g1.try_reserve(1));
            g1.release(1);
        });

        let g2 = guard.clone();
        let t2 = thread::spawn(move || {
            assert!(g2.try_reserve(1));
            g2.release(1);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        assert_eq!(guard.inflight(), 0, "all credits released — inflight must be 0");
    });
}
