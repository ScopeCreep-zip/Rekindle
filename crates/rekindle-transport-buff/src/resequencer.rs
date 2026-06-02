//! [`Resequencer`] — composed dispatch→parallel→reorder unit.
//!
//! Combines a [`DispatchQueue`] (fan-out to workers) and a [`ReorderRing`]
//! (reassemble in order) into a single structure that enforces the
//! **nonce-before-dispatch invariant** structurally:
//!
//! - [`submit`](Resequencer::submit) fixes `seq` before any worker sees the item.
//! - [`take_work`](Resequencer::take_work) returns `(seq, input)` — the worker
//!   receives the seq, it cannot assign one.
//! - [`complete`](Resequencer::complete) publishes the output at the original seq.
//! - [`drain`](Resequencer::drain) delivers the contiguous prefix in order.
//!
//! There is **no API** by which a worker assigns or changes a sequence number.
//! The invariant is enforced by the shape of the type, not by documentation.
//!
//! # Worker panic safety
//!
//! [`take_work`](Resequencer::take_work) returns a [`WorkGuard`] that is
//! `#[must_use]`. If the guard is dropped without calling
//! [`complete`](WorkGuard::complete) (e.g., the rayon worker panics), the guard
//! publishes a [`WorkerFailed`] sentinel into the ring so the consumer's
//! [`drain`](Resequencer::drain) can advance past the gap. Without this, a
//! worker panic would stall the ring at that seq forever.
//!
//! # Replaces
//!
//! - The `BulkSender` nonce-assignment-before-dispatch dance in
//!   `v3/bulk/send.rs`
//! - The audit reorder + dispatch pipeline in `v3/io/control_loop/`

use core::fmt;
use std::sync::Arc;

use crate::dispatch::DispatchQueue;
use crate::reorder::{PublishError, ReorderRing};
use crate::traits::{SpinWake, WakeSink};

// ---------------------------------------------------------------------------
// WorkerFailed sentinel
// ---------------------------------------------------------------------------

/// Sentinel published into the [`ReorderRing`] when a worker panics
/// (the [`WorkGuard`] is dropped without calling [`complete`](WorkGuard::complete)).
///
/// The consumer's [`drain`](Resequencer::drain) callback receives
/// `Result<U, WorkerFailed>` — it can distinguish a successful output
/// from a panic-gap sentinel and act accordingly (e.g., NACK the chunk,
/// close the session).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerFailed;

impl fmt::Display for WorkerFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("worker panicked or dropped WorkGuard without completing")
    }
}

impl std::error::Error for WorkerFailed {}

// ---------------------------------------------------------------------------
// Resequencer
// ---------------------------------------------------------------------------

/// Composed dispatch→parallel→reorder unit.
///
/// `T` is the input type dispatched to workers.
/// `U` is the output type workers produce after processing.
/// `W` is the [`WakeSink`] implementation for consumer notification.
///
/// The ring stores `Result<U, WorkerFailed>` so the consumer can
/// distinguish successful outputs from panic-gap sentinels.
pub struct Resequencer<T, U, W: WakeSink = SpinWake> {
    dispatch: DispatchQueue<T, W>,
    reorder: Arc<ReorderRing<Result<U, WorkerFailed>>>,
    drain_wake: W,
}

// Send + Sync: DispatchQueue is Send+Sync when T: Send, W: Send+Sync.
// Arc<ReorderRing<Result<U, WorkerFailed>>> is Send+Sync when U: Send.
// W: WakeSink requires Send + Sync + Clone + 'static.
// No additional unsafe impls needed — the auto-impls apply correctly.

impl<T, U, W: WakeSink> Resequencer<T, U, W> {
    /// Create a resequencer.
    ///
    /// - `queue_cap`: capacity of the dispatch queue (bounded MPMC).
    /// - `window`: capacity of the reorder ring (must be power of two).
    /// - `push_wake`: wake sink called when items are pushed to the dispatch queue.
    /// - `drain_wake`: wake sink called when items are published to the reorder ring.
    ///
    /// # Panics
    ///
    /// Panics if `queue_cap == 0` or `window` is not a power of two.
    pub fn new(queue_cap: usize, window: usize, push_wake: W, drain_wake: W) -> Self {
        Self {
            dispatch: DispatchQueue::new(queue_cap, push_wake),
            reorder: Arc::new(ReorderRing::new(window)),
            drain_wake,
        }
    }

    /// Producer: submit an input at a consumer-assigned `seq`.
    ///
    /// The `seq` is fixed **here**, before any worker sees the item.
    /// There is no API by which a worker assigns seq — this is the
    /// nonce-before-dispatch invariant made structural.
    ///
    /// Returns `Err(ResequencerFull)` if the dispatch queue is full
    /// (backpressure signal — the consumer awaits).
    pub fn submit(&self, seq: u64, input: T) -> Result<(), ResequencerFull<T>> {
        self.dispatch
            .try_push(seq, input)
            .map_err(|(seq, item)| ResequencerFull { seq, item })
    }

    /// Worker: take one unit of work.
    ///
    /// Returns a [`WorkGuard`] that must be completed via
    /// [`WorkGuard::complete`]. If the guard is dropped without
    /// completing (e.g., panic), it publishes a [`WorkerFailed`]
    /// sentinel so the consumer can advance past the gap.
    ///
    /// Many workers call this concurrently (work-stealing-friendly).
    /// Returns `None` if the dispatch queue is empty.
    pub fn take_work(&self) -> Option<WorkGuard<T, U, W>> {
        let (seq, input) = self.dispatch.pop()?;
        Some(WorkGuard {
            reorder: Arc::clone(&self.reorder),
            drain_wake: self.drain_wake.clone(),
            seq,
            input: Some(input),
            completed: false,
        })
    }

    /// Consumer (single): drain the reassembled contiguous prefix in order.
    ///
    /// The callback receives `(seq, Result<U, WorkerFailed>)`. On
    /// `Ok(output)`, the worker completed successfully. On
    /// `Err(WorkerFailed)`, the worker panicked — the consumer decides
    /// what this means (NACK, close session, skip, etc.).
    ///
    /// Returns the number of items delivered (including sentinels).
    pub fn drain(&self, f: impl FnMut(u64, Result<U, WorkerFailed>)) -> usize {
        self.reorder.drain_contiguous(f)
    }

    /// The next sequence number the consumer expects to deliver.
    pub fn next_deliver(&self) -> u64 {
        self.reorder.next_deliver()
    }

    /// The dispatch queue's current length (items waiting for workers).
    pub fn dispatch_len(&self) -> usize {
        self.dispatch.len()
    }

    /// The dispatch queue's capacity.
    pub fn dispatch_capacity(&self) -> usize {
        self.dispatch.capacity()
    }

    /// Whether the dispatch queue is empty.
    pub fn dispatch_is_empty(&self) -> bool {
        self.dispatch.is_empty()
    }

    /// The reorder ring's window size.
    pub fn window(&self) -> usize {
        self.reorder.window()
    }

    /// Access the underlying reorder ring (for observability).
    pub fn reorder_ring(&self) -> &ReorderRing<Result<U, WorkerFailed>> {
        &self.reorder
    }
}

impl<T: fmt::Debug, U: fmt::Debug, W: WakeSink> fmt::Debug for Resequencer<T, U, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resequencer")
            .field("dispatch_len", &self.dispatch.len())
            .field("dispatch_cap", &self.dispatch.capacity())
            .field("next_deliver", &self.reorder.next_deliver())
            .field("window", &self.reorder.window())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// WorkGuard — RAII guard for in-progress work
// ---------------------------------------------------------------------------

/// RAII guard for a work item taken from the [`Resequencer`].
///
/// **Must** be completed via [`complete`](WorkGuard::complete). If dropped
/// without completing (panic, early return), publishes a [`WorkerFailed`]
/// sentinel into the reorder ring so the consumer can advance past the gap.
///
/// # `#[must_use]`
///
/// Ignoring a `WorkGuard` (e.g., `let _ = resequencer.take_work()`) silently
/// publishes a failure sentinel. The `#[must_use]` attribute makes this a
/// compiler warning.
///
/// # Not `Clone`
///
/// Two guards for the same seq would double-publish. `WorkGuard` does not
/// implement `Clone`.
/// `WorkGuard` holds `Arc<ReorderRing>` and a cloned `W` — not a borrow
/// of the `Resequencer`. This makes `WorkGuard: Send` when `T: Send` and
/// `U: Send`, so it can be moved into a rayon closure (`rayon::spawn`
/// requires `'static + Send`). The `Arc` clone cost is one
/// `fetch_add(Relaxed)` — identical to `crossbeam::Sender::clone`.
#[must_use = "dropping a WorkGuard without calling complete() publishes a failure sentinel"]
pub struct WorkGuard<T, U, W: WakeSink = SpinWake> {
    reorder: Arc<ReorderRing<Result<U, WorkerFailed>>>,
    drain_wake: W,
    seq: u64,
    input: Option<T>,
    completed: bool,
}

impl<T, U, W: WakeSink> WorkGuard<T, U, W> {
    /// The sequence number assigned to this work item.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Access the input data.
    ///
    /// Returns `None` if `take_input()` was already called.
    pub fn input(&self) -> Option<&T> {
        self.input.as_ref()
    }

    /// Take ownership of the input data.
    ///
    /// This is useful when the worker needs to move the input into a
    /// processing function. Can only be called once.
    pub fn take_input(&mut self) -> Option<T> {
        self.input.take()
    }

    /// Complete the work and publish the output into the reorder ring.
    ///
    /// Consumes the guard. The output is published at the seq that was
    /// assigned when the work was submitted — not a seq the worker chose.
    ///
    /// Sets `completed = true` so that `Drop` does not publish the
    /// sentinel. No `mem::forget` needed — `Drop` runs normally and
    /// checks the flag. All owned fields (`Arc`, `W`) are dropped
    /// normally, decrementing refcounts correctly.
    ///
    /// # Errors
    ///
    /// Returns `Err(PublishError)` if the reorder ring rejects the publish
    /// (overflow or slot occupied). This is a bug in the consumer's
    /// sequencing discipline, not a normal error.
    pub fn complete(mut self, output: U) -> Result<(), PublishError<Result<U, WorkerFailed>>> {
        let result = self.reorder.publish(self.seq, Ok(output));
        if result.is_ok() {
            self.completed = true;
            self.drain_wake.wake();
        }
        // If publish succeeded: completed == true, Drop is a no-op.
        // If publish failed: completed == false, Drop publishes
        // Err(WorkerFailed) sentinel so the consumer can advance.
        result
    }
}

impl<T, U, W: WakeSink> Drop for WorkGuard<T, U, W> {
    fn drop(&mut self) {
        if !self.completed {
            // Publish the WorkerFailed sentinel so the consumer can
            // advance past this seq.
            //
            // This publish is structurally guaranteed to succeed:
            // - The seq was assigned by the consumer at submit() time
            //   (the nonce-before-dispatch invariant).
            // - The slot for self.seq is within the window (it was
            //   within the window at submit time and the consumer has
            //   not advanced past it — the consumer cannot drain past
            //   a seq that has not been published).
            // - No other WorkGuard holds the same seq (WorkGuard is
            //   not Clone, and take_work() pops from the DispatchQueue
            //   which guarantees each (seq, input) pair is consumed
            //   exactly once).
            //
            // Therefore Overflow and Occupied are unreachable. If they
            // somehow occur, it is a bug in the Resequencer's seq
            // discipline, not a recoverable condition. We intentionally
            // do NOT debug_assert here because Drop can fire during
            // panic unwind — a debug_assert failure inside Drop during
            // unwind is a double-panic, which Rust converts to abort().
            // The sentinel is our best effort; if it fails, the consumer
            // stalls, which surfaces the bug as a hang rather than a
            // silent abort with no diagnostics.
            let _ = self.reorder.publish(self.seq, Err(WorkerFailed));
            self.drain_wake.wake();
        }
    }
}

// ---------------------------------------------------------------------------
// ResequencerFull — backpressure error
// ---------------------------------------------------------------------------

/// Error returned by [`Resequencer::submit`] when the dispatch queue is full.
///
/// Contains the original `seq` and `item` so the caller can retry.
#[derive(Debug)]
pub struct ResequencerFull<T> {
    /// The sequence number that was not submitted.
    pub seq: u64,
    /// The input item returned to the caller.
    pub item: T,
}

impl<T> fmt::Display for ResequencerFull<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "resequencer dispatch queue full at seq {}", self.seq)
    }
}

impl<T: fmt::Debug> std::error::Error for ResequencerFull<T> {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use crate::traits::SpinWake;

    #[test]
    fn submit_take_complete_drain() {
        let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);

        rs.submit(0, 100).unwrap();
        rs.submit(1, 200).unwrap();

        let mut work0 = rs.take_work().unwrap();
        let input0 = work0.take_input().unwrap();
        assert_eq!(input0, 100);
        work0.complete(input0 * 10).unwrap();

        let mut work1 = rs.take_work().unwrap();
        let input1 = work1.take_input().unwrap();
        assert_eq!(input1, 200);
        work1.complete(input1 * 10).unwrap();

        let mut delivered = Vec::new();
        rs.drain(|seq, result| delivered.push((seq, result)));
        assert_eq!(delivered, vec![
            (0, Ok(1000)),
            (1, Ok(2000)),
        ]);
    }

    #[test]
    fn out_of_order_complete_delivers_in_order() {
        let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);

        rs.submit(0, 10).unwrap();
        rs.submit(1, 20).unwrap();
        rs.submit(2, 30).unwrap();

        // Take all three.
        let mut w0 = rs.take_work().unwrap();
        let mut w1 = rs.take_work().unwrap();
        let mut w2 = rs.take_work().unwrap();

        // Complete out of order: 2, 0, 1.
        let i2 = w2.take_input().unwrap();
        w2.complete(i2).unwrap();

        let i0 = w0.take_input().unwrap();
        w0.complete(i0).unwrap();

        // Drain: only 0 is contiguous from next_deliver=0.
        let mut delivered = Vec::new();
        rs.drain(|seq, result| delivered.push((seq, result)));
        assert_eq!(delivered, vec![(0, Ok(10))]);

        // Complete 1 — now 1 and 2 are contiguous.
        let i1 = w1.take_input().unwrap();
        w1.complete(i1).unwrap();

        let mut delivered = Vec::new();
        rs.drain(|seq, result| delivered.push((seq, result)));
        assert_eq!(delivered, vec![(1, Ok(20)), (2, Ok(30))]);
    }

    #[test]
    fn worker_panic_publishes_sentinel() {
        let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);

        rs.submit(0, 100).unwrap();
        rs.submit(1, 200).unwrap();
        rs.submit(2, 300).unwrap();

        // Take seq 0, complete it.
        let mut w0 = rs.take_work().unwrap();
        let input0 = w0.take_input().unwrap();
        w0.complete(input0).unwrap();

        // Take seq 1 — drop without completing (simulates panic).
        {
            let _w1 = rs.take_work().unwrap();
            // dropped here → publishes WorkerFailed sentinel
        }

        // Take seq 2, complete it.
        let mut w2 = rs.take_work().unwrap();
        let input2 = w2.take_input().unwrap();
        w2.complete(input2).unwrap();

        // Drain: should see all three, with seq 1 as WorkerFailed.
        let mut delivered = Vec::new();
        rs.drain(|seq, result| delivered.push((seq, result)));
        assert_eq!(delivered, vec![
            (0, Ok(100)),
            (1, Err(WorkerFailed)),
            (2, Ok(300)),
        ]);
    }

    #[test]
    fn submit_full_returns_item() {
        let rs = Resequencer::<u64, u64>::new(2, 8, SpinWake, SpinWake);

        rs.submit(0, 10).unwrap();
        rs.submit(1, 20).unwrap();
        let err = rs.submit(2, 30).unwrap_err();
        assert_eq!(err.seq, 2);
        assert_eq!(err.item, 30);
    }

    #[test]
    fn take_work_returns_none_when_empty() {
        let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);
        assert!(rs.take_work().is_none());
    }

    #[test]
    fn concurrent_workers() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;

        let rs = Arc::new(Resequencer::<u64, u64>::new(64, 64, SpinWake, SpinWake));
        let completed = Arc::new(AtomicUsize::new(0));
        let total = 32usize;

        // Submit all items before workers start.
        for i in 0..total as u64 {
            rs.submit(i, i * 100).unwrap();
        }

        // 4 worker threads, each takes and completes work until all 32 are done.
        // A worker may get zero items (work-stealing is unfair) — the exit
        // condition is the global completion count, not a per-worker count.
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let rs = Arc::clone(&rs);
                let completed = Arc::clone(&completed);
                thread::spawn(move || {
                    loop {
                        if completed.load(Ordering::Acquire) >= total {
                            break;
                        }
                        match rs.take_work() {
                            Some(mut guard) => {
                                let input = guard.take_input().unwrap();
                                guard.complete(input + 1).unwrap();
                                completed.fetch_add(1, Ordering::Release);
                            }
                            None => {
                                std::thread::yield_now();
                            }
                        }
                    }
                })
            })
            .collect();

        for w in workers {
            w.join().unwrap();
        }

        // Drain — all 32 items should be delivered in order.
        let mut delivered = Vec::new();
        rs.drain(|seq, result| {
            let val = result.unwrap();
            delivered.push((seq, val));
        });

        assert_eq!(delivered.len(), 32);
        for (i, (seq, val)) in delivered.iter().enumerate() {
            assert_eq!(*seq, i as u64);
            assert_eq!(*val, i as u64 * 100 + 1);
        }
    }

    #[test]
    fn work_guard_is_send() {
        // WorkGuard must be Send when T: Send and U: Send — this is the
        // rayon use case. rayon::spawn requires 'static + Send. The guard
        // holds Arc<ReorderRing> (Send) and a cloned W (Send), not a
        // borrow, so it satisfies both bounds.
        fn assert_send<T: Send>() {}
        assert_send::<WorkGuard<u64, u64, SpinWake>>();
    }

    #[test]
    fn work_guard_cross_thread_complete() {
        // Prove the guard can actually be moved to another thread and
        // completed there — the production rayon pattern.
        use std::sync::Arc;
        use std::thread;

        let rs = Arc::new(Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake));

        rs.submit(0, 42).unwrap();
        let guard = rs.take_work().unwrap();

        // Move the guard to another thread and complete it there.
        let handle = thread::spawn(move || {
            let mut g = guard;
            let input = g.take_input().unwrap();
            g.complete(input + 1).unwrap();
        });
        handle.join().unwrap();

        let mut delivered = Vec::new();
        rs.drain(|seq, result| delivered.push((seq, result)));
        assert_eq!(delivered, vec![(0, Ok(43))]);
    }

    #[test]
    fn debug_format() {
        let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);
        let debug = format!("{rs:?}");
        assert!(debug.contains("Resequencer"));
        assert!(debug.contains("dispatch_len"));
    }
}
