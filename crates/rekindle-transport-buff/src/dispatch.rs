//! [`DispatchQueue`] — bounded MPMC fan-out to workers.
//!
//! A thin, opinionated wrapper over [`ArrayQueue<(u64, T)>`](crossbeam_queue::ArrayQueue)
//! that bakes in the established choices so a consumer cannot get them wrong:
//! bounded capacity (backpressure), `(seq, payload)` pairing (the downstream
//! [`ReorderRing`](crate::ReorderRing) needs the seq to reassemble in order),
//! and a [`WakeSink`] call after every successful push.
//!
//! # Replaces
//!
//! - `crossbeam::channel::bounded` wrappers in `v3/io/lane_channels.rs`
//! - The crypto-dispatch channels in `v3/bulk/send.rs` and `v3/bulk/recv.rs`
//!
//! # What this queue does NOT do
//!
//! - Does not assign sequence numbers. The consumer assigns `seq` before push.
//! - Does not interpret backpressure. A full queue returns `Err` with the item.
//!   The caller decides whether to block, drop, or nack.
//! - Does not own any wake mechanism. It calls `WakeSink::wake()` after a
//!   successful push — the wake implementation is the consumer's.

use core::fmt;
use core::panic::{RefUnwindSafe, UnwindSafe};

use crossbeam_queue::ArrayQueue;

use crate::traits::{SpinWake, WakeSink};

// ---------------------------------------------------------------------------
// DispatchQueue
// ---------------------------------------------------------------------------

/// Bounded MPMC dispatch queue. Carries `(seq, payload)` so downstream
/// reorder structures know which sequence each item belongs to.
///
/// Push is non-blocking: returns `Err((seq, item))` when full (the
/// backpressure signal). Pop is non-blocking: returns `None` when empty.
///
/// # Type parameters
///
/// - `T`: the payload. Typically a sealed job (send) or a ciphertext chunk (recv).
/// - `W`: [`WakeSink`] — called after every successful push to notify the
///   consumer that work is available. Defaults to [`SpinWake`] (no-op).
pub struct DispatchQueue<T, W: WakeSink = SpinWake> {
    /// The backing bounded MPMC queue. `ArrayQueue` is Vyukov-stamped with
    /// `CachePadded` head/tail — no additional padding needed here.
    inner: ArrayQueue<(u64, T)>,
    /// Wake signal sent to the consumer after each successful push.
    wake: W,
}

// Send + Sync: ArrayQueue<(u64, T)> is Send + Sync when T: Send.
// W: WakeSink requires Send + Sync. No additional bounds needed.

// Explicit UnwindSafe impls. ArrayQueue<T> is unconditionally UnwindSafe
// (crossbeam array_queue.rs:79-80). W: WakeSink does not bound UnwindSafe,
// so the auto-impl would fail for W types that aren't UnwindSafe. A
// DispatchQueue is safe to observe across a catch_unwind boundary because
// its state is entirely in the lock-free ArrayQueue — no invariant is
// violated by a panic between push and pop.
impl<T, W: WakeSink> UnwindSafe for DispatchQueue<T, W> {}
impl<T, W: WakeSink> RefUnwindSafe for DispatchQueue<T, W> {}

impl<T, W: WakeSink> DispatchQueue<T, W> {
    /// Create a dispatch queue with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero (crossbeam `ArrayQueue::new` panics).
    pub fn new(capacity: usize, wake: W) -> Self {
        Self {
            inner: ArrayQueue::new(capacity),
            wake,
        }
    }

    /// Non-blocking push. Returns `Ok(())` on success, `Err((seq, item))`
    /// when the queue is full.
    ///
    /// On success, calls `WakeSink::wake()` to notify the consumer that
    /// work is available. The wake call is non-blocking and idempotent.
    ///
    /// The returned `Err` contains the original `(seq, item)` — nothing
    /// is silently dropped.
    #[inline]
    pub fn try_push(&self, seq: u64, item: T) -> Result<(), (u64, T)> {
        match self.inner.push((seq, item)) {
            Ok(()) => {
                self.wake.wake();
                Ok(())
            }
            Err(val) => Err(val),
        }
    }

    /// Non-blocking pop. Returns `Some((seq, item))` if available,
    /// `None` if the queue is empty.
    ///
    /// Many workers call this concurrently — `ArrayQueue::pop` is MPMC.
    /// The `seq` in the returned tuple is the sequence number the
    /// downstream [`ReorderRing`](crate::ReorderRing) or
    /// [`Resequencer`](crate::Resequencer) needs for reassembly.
    #[inline]
    pub fn pop(&self) -> Option<(u64, T)> {
        self.inner.pop()
    }

    /// Current number of items in the queue. Point-in-time snapshot —
    /// may be stale under concurrent access. Diagnostic only; do not
    /// use for scheduling decisions (use `try_push` / `pop` results).
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// The queue's capacity. Immutable after construction.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Whether the queue appears empty. Point-in-time snapshot.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether the queue appears full. Point-in-time snapshot.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    /// Access the wake sink.
    #[inline]
    pub fn wake_sink(&self) -> &W {
        &self.wake
    }
}

impl<T, W: WakeSink> fmt::Debug for DispatchQueue<T, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DispatchQueue")
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use crate::traits::SpinWake;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use std::sync::Arc;

    #[test]
    fn push_and_pop_in_order() {
        let q = DispatchQueue::new(8, SpinWake);
        q.try_push(0, 'a').unwrap();
        q.try_push(1, 'b').unwrap();
        q.try_push(2, 'c').unwrap();

        assert_eq!(q.pop(), Some((0, 'a')));
        assert_eq!(q.pop(), Some((1, 'b')));
        assert_eq!(q.pop(), Some((2, 'c')));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn full_returns_err_with_item() {
        let q = DispatchQueue::new(2, SpinWake);
        q.try_push(0, 10u64).unwrap();
        q.try_push(1, 20u64).unwrap();

        let err = q.try_push(2, 30u64).unwrap_err();
        assert_eq!(err, (2, 30u64));
    }

    #[test]
    fn empty_pop_returns_none() {
        let q = DispatchQueue::<u64>::new(4, SpinWake);
        assert!(q.is_empty());
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn len_and_capacity() {
        let q = DispatchQueue::new(8, SpinWake);
        assert_eq!(q.capacity(), 8);
        assert_eq!(q.len(), 0);

        q.try_push(0, 42u64).unwrap();
        assert_eq!(q.len(), 1);
        assert!(!q.is_empty());
        assert!(!q.is_full());
    }

    #[test]
    fn wake_called_on_push() {
        // Local counter per test — no shared static across parallel tests.
        let count = Arc::new(AtomicUsize::new(0));

        #[derive(Clone)]
        struct CountingWake(Arc<AtomicUsize>);
        impl WakeSink for CountingWake {
            fn wake(&self) {
                self.0.fetch_add(1, Relaxed);
            }
        }

        let q = DispatchQueue::new(8, CountingWake(Arc::clone(&count)));
        q.try_push(0, 'a').unwrap();
        q.try_push(1, 'b').unwrap();
        assert_eq!(count.load(Relaxed), 2);
    }

    #[test]
    fn wake_not_called_on_full() {
        let count = Arc::new(AtomicUsize::new(0));

        #[derive(Clone)]
        struct CountingWake(Arc<AtomicUsize>);
        impl WakeSink for CountingWake {
            fn wake(&self) {
                self.0.fetch_add(1, Relaxed);
            }
        }

        let q = DispatchQueue::new(1, CountingWake(Arc::clone(&count)));
        q.try_push(0, 'a').unwrap();
        assert_eq!(count.load(Relaxed), 1);
        let _ = q.try_push(1, 'b'); // full — no wake
        assert_eq!(count.load(Relaxed), 1);
    }

    #[test]
    fn concurrent_mpmc() {
        use std::collections::HashSet;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;

        let q = Arc::new(DispatchQueue::new(64, SpinWake));
        let done = Arc::new(AtomicBool::new(false));

        // 4 producers, each pushes 100 items with globally unique seqs.
        let producers: Vec<_> = (0..4)
            .map(|p| {
                let q = Arc::clone(&q);
                thread::spawn(move || {
                    for i in 0..100u64 {
                        let seq = p * 100 + i;
                        while q.try_push(seq, seq).is_err() {
                            std::hint::spin_loop();
                        }
                    }
                })
            })
            .collect();

        // 4 consumers, each collects popped items until producers are done
        // AND the queue is empty. A consumer may get zero items (work-stealing
        // is unfair) — the exit condition must not assume each gets at least one.
        let consumers: Vec<_> = (0..4)
            .map(|_| {
                let q = Arc::clone(&q);
                let done = Arc::clone(&done);
                thread::spawn(move || {
                    let mut collected = Vec::new();
                    loop {
                        match q.pop() {
                            Some((seq, val)) => {
                                assert_eq!(seq, val, "seq/val mismatch — data corruption");
                                collected.push(seq);
                            }
                            None => {
                                if done.load(Ordering::Acquire) {
                                    break;
                                }
                                std::thread::yield_now();
                            }
                        }
                    }
                    collected
                })
            })
            .collect();

        for p in producers {
            p.join().unwrap();
        }
        done.store(true, Ordering::Release);

        // Collect all items: from consumers + any stragglers in the queue.
        let mut all: HashSet<u64> = HashSet::new();
        for c in consumers {
            for seq in c.join().unwrap() {
                let is_new = all.insert(seq);
                assert!(is_new, "duplicate pop detected: seq {seq}");
            }
        }
        while let Some((seq, _)) = q.pop() {
            let is_new = all.insert(seq);
            assert!(is_new, "duplicate pop detected in straggler drain: seq {seq}");
        }

        assert_eq!(all.len(), 400, "expected 400 unique items from 4×100");
    }

    #[test]
    fn debug_format() {
        let q = DispatchQueue::new(16, SpinWake);
        q.try_push(0, 'a').unwrap();
        let debug = format!("{q:?}");
        assert!(debug.contains("DispatchQueue"));
        assert!(debug.contains("len: 1"));
        assert!(debug.contains("capacity: 16"));
    }

    #[test]
    #[should_panic]
    fn zero_capacity_panics() {
        let _q = DispatchQueue::<u64>::new(0, SpinWake);
    }
}
