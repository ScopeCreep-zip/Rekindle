#![cfg(not(loom))]
//! Scaling regression gate.
//!
//! Detects catastrophic throughput collapse caused by false sharing,
//! contention bugs, or broken cache-line discipline in the lock-free
//! primitives. These are `#[test]` functions that run as part of
//! `cargo test` and FAIL if the scaling shape is wrong. The criterion
//! benches in `benches/` produce detailed reports for investigation;
//! these tests produce pass/fail for CI.
//!
//! # Measurement discipline
//!
//! Each thread count gets a warmup pass (discarded) before the measured
//! pass. This stabilizes CPU thermal state and cache residency so that
//! comparison across thread counts reflects scaling behavior, not thermal
//! ramp from cold→hot across sequential runs.
//!
//! Best-of-3 measured passes per thread count absorbs transient OS
//! scheduling noise (migration, timer interrupts, compaction).
//!
//! # Gates
//!
//! Two gates, both must pass:
//!
//! - **Pairwise**: throughput at N threads must be at least `PAIRWISE_FLOOR`
//!   of throughput at N/2 threads. Detects collapse at a specific thread
//!   count (the false-sharing signature: fine at 1, fine at 2, collapses
//!   at 4 when adjacent slots share a cache line).
//!
//! - **Absolute**: throughput at N threads must be at least `ABSOLUTE_FLOOR`
//!   of the single-thread baseline. Catches gradual decay that pairwise
//!   misses (each step loses 40% → pairwise passes, but absolute ratio
//!   at 8 threads is 0.04).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rekindle_transport_buff::{CreditGuard, DispatchQueue, ReorderRing, SpinWake};

/// Pairwise floor: N-thread throughput must be >= this fraction of (N/2)-thread.
/// 0.25 allows a 4x drop per doubling (severe but not catastrophic). A real
/// false-sharing regression shows as 10-50x collapse at the affected level.
const PAIRWISE_FLOOR: f64 = 0.25;

/// Absolute floor against warm baseline. 0.05 (20x allowed total decay from
/// 1→8 threads). Catches gradual compound decay that pairwise misses.
/// CreditGuard's single-counter CAS is expected to hit ~0.15-0.25 at 8
/// threads on 4P/8L — 0.05 catches only true catastrophic failure.
const ABSOLUTE_FLOOR: f64 = 0.05;

/// Warmup iterations before measurement. Stabilizes thermal state.
const WARMUP_ROUNDS: usize = 1;
/// Measured iterations — best-of-N absorbs transient scheduling noise.
const MEASURE_ROUNDS: usize = 3;

fn ops_per_sec(items: u64, elapsed: Duration) -> f64 {
    items as f64 / elapsed.as_secs_f64()
}

// ---------------------------------------------------------------------------
// Shared measurement harness
// ---------------------------------------------------------------------------

/// Run `workload` for warmup, then take the best throughput from N measured
/// runs. `workload(thread_count)` returns the total item count processed.
fn measure_throughput(
    thread_counts: &[usize],
    workload: impl Fn(usize) -> u64,
) -> Vec<(usize, f64)> {
    let mut results = Vec::with_capacity(thread_counts.len());

    for &n in thread_counts {
        // Warmup: run the workload at this thread count to stabilize
        // CPU frequency, cache residency, and thermal state.
        for _ in 0..WARMUP_ROUNDS {
            let _ = workload(n);
        }

        // Measure: best-of-N. Take the highest throughput — the run
        // least affected by OS noise is the most representative of
        // the primitive's actual scaling behavior.
        let mut best = 0.0f64;
        for _ in 0..MEASURE_ROUNDS {
            let start = Instant::now();
            let items = workload(n);
            let elapsed = start.elapsed();
            let throughput = ops_per_sec(items, elapsed);
            if throughput > best {
                best = throughput;
            }
        }

        results.push((n, best));
    }

    results
}

/// Assert pairwise and absolute gates on measurement results.
fn assert_scaling(label: &str, results: &[(usize, f64)]) {
    let baseline = results[0].1;

    // Absolute gate: every thread count vs baseline.
    for &(n, throughput) in results {
        let ratio = throughput / baseline;
        assert!(
            ratio >= ABSOLUTE_FLOOR,
            "{label} ABSOLUTE scaling failure at {n} threads: \
             {throughput:.0} ops/s vs baseline {baseline:.0} ops/s \
             (ratio {ratio:.3}, floor {ABSOLUTE_FLOOR})"
        );
    }

    // Pairwise gate: each level vs its predecessor.
    for i in 1..results.len() {
        let (prev_n, prev_tp) = results[i - 1];
        let (n, throughput) = results[i];
        let ratio = throughput / prev_tp;
        assert!(
            ratio >= PAIRWISE_FLOOR,
            "{label} PAIRWISE scaling failure at {n} threads vs {prev_n} threads: \
             {throughput:.0} ops/s vs {prev_tp:.0} ops/s \
             (ratio {ratio:.3}, floor {PAIRWISE_FLOOR})"
        );
    }
}

// ---------------------------------------------------------------------------
// ReorderRing scaling: 1, 2, 4, 8 producers, 1 consumer
// ---------------------------------------------------------------------------

#[test]
fn reorder_ring_scaling_not_retrograde() {
    const WINDOW: usize = 1024;
    const ITEMS_PER_PRODUCER: u64 = 50_000;

    let results = measure_throughput(&[1, 2, 4, 8], |producers| {
        let total = ITEMS_PER_PRODUCER * producers as u64;
        let ring = Arc::new(ReorderRing::<u64>::new(WINDOW));

        let handles: Vec<_> = (0..producers)
            .map(|p| {
                let r = ring.clone();
                let s = p as u64 * ITEMS_PER_PRODUCER;
                let e = s + ITEMS_PER_PRODUCER;
                thread::spawn(move || {
                    for seq in s..e {
                        while r.publish(seq, seq).is_err() {
                            std::hint::spin_loop();
                        }
                    }
                })
            })
            .collect();

        let mut delivered = 0u64;
        while delivered < total {
            delivered += ring.drain_contiguous(|_, _| {}) as u64;
            if delivered < total {
                std::hint::spin_loop();
            }
        }

        for h in handles {
            h.join().unwrap();
        }

        total
    });

    assert_scaling("ReorderRing", &results);
}

// ---------------------------------------------------------------------------
// DispatchQueue scaling: 1 producer, 1/2/4/8 workers
// ---------------------------------------------------------------------------

#[test]
fn dispatch_queue_scaling_not_retrograde() {
    const CAPACITY: usize = 256;
    const TOTAL_ITEMS: u64 = 100_000;

    let results = measure_throughput(&[1, 2, 4, 8], |workers| {
        let queue = Arc::new(DispatchQueue::<u64>::new(CAPACITY, SpinWake));
        let done = Arc::new(AtomicBool::new(false));

        let worker_handles: Vec<_> = (0..workers)
            .map(|_| {
                let q = queue.clone();
                let d = done.clone();
                thread::spawn(move || {
                    let mut count = 0u64;
                    loop {
                        if q.pop().is_some() {
                            count += 1;
                        } else if d.load(Ordering::Acquire) {
                            // Drain remaining after done signal.
                            while q.pop().is_some() {
                                count += 1;
                            }
                            break;
                        } else {
                            std::hint::spin_loop();
                        }
                    }
                    count
                })
            })
            .collect();

        for seq in 0..TOTAL_ITEMS {
            while queue.try_push(seq, seq).is_err() {
                std::hint::spin_loop();
            }
        }

        done.store(true, Ordering::Release);

        let total_popped: u64 = worker_handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .sum();

        // Drain stragglers (workers may exit between done check and final pop).
        let mut stragglers = 0u64;
        while queue.pop().is_some() {
            stragglers += 1;
        }

        assert_eq!(
            total_popped + stragglers,
            TOTAL_ITEMS,
            "lost items at {workers} workers"
        );

        TOTAL_ITEMS
    });

    assert_scaling("DispatchQueue", &results);
}

// ---------------------------------------------------------------------------
// CreditGuard contention: 1/2/4/8 threads on one counter
// ---------------------------------------------------------------------------

#[test]
fn credit_guard_contention_not_catastrophic() {
    const OPS_PER_THREAD: u64 = 200_000;

    let results = measure_throughput(&[1, 2, 4, 8], |threads| {
        let total = OPS_PER_THREAD * threads as u64;
        let guard = Arc::new(CreditGuard::new(threads as u64 * OPS_PER_THREAD));

        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let g = guard.clone();
                thread::spawn(move || {
                    for _ in 0..OPS_PER_THREAD {
                        assert!(g.try_reserve(1));
                        g.release(1);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(guard.inflight(), 0, "credit leak at {threads} threads");

        total
    });

    assert_scaling("CreditGuard", &results);
}
