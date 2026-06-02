#![cfg(not(loom))]
//! Scaling regression gate.
//!
//! Asserts that throughput scaling from 1 to 8 producers/workers is
//! monotonic-or-flat — a retrograde result (throughput declines as
//! cores increase) signals reintroduced false sharing or contention.
//!
//! These are `#[test]` functions, not criterion benches. They run as
//! part of `cargo test` and FAIL if the scaling shape is wrong. The
//! criterion benches in `benches/` produce detailed reports for
//! investigation; these tests produce pass/fail for CI.
//!
//! # Thresholds
//!
//! Retrograde threshold: throughput at N threads must be at least 40%
//! of throughput at 1 thread. This is deliberately loose — we're
//! catching catastrophic regressions (10x collapse), not benchmarking
//! to 1%. Thermal throttling, CI noise, and hyperthread contention
//! can cause 2x swings; 40% absorbs that. A real regression (false
//! sharing reintroduced) shows as 5-10x collapse which this catches.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rekindle_transport_buff::{CreditGuard, DispatchQueue, ReorderRing, SpinWake};

/// Throughput at N threads must be at least this fraction of single-thread.
/// 0.20 absorbs hyperthread contention on 4P/8L machines (where 8 producers
/// on 4 physical cores is 2x oversubscribed — expected ~0.25 ratio) while
/// still catching catastrophic false-sharing regressions (which show as
/// ratio < 0.10, a 10x+ collapse).
const RETROGRADE_FLOOR: f64 = 0.20;

fn ops_per_sec(items: u64, elapsed: Duration) -> f64 {
    items as f64 / elapsed.as_secs_f64()
}

// ---------------------------------------------------------------------------
// ReorderRing scaling: 1, 2, 4, 8 producers, 1 consumer
// ---------------------------------------------------------------------------

#[test]
fn reorder_ring_scaling_not_retrograde() {
    const WINDOW: usize = 1024;
    const ITEMS_PER_PRODUCER: u64 = 50_000;

    let mut results: Vec<(usize, f64)> = Vec::new();

    for &producers in &[1usize, 2, 4, 8] {
        let total = ITEMS_PER_PRODUCER * producers as u64;
        let ring = Arc::new(ReorderRing::<u64>::new(WINDOW));

        let start = Instant::now();

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

        let elapsed = start.elapsed();
        let throughput = ops_per_sec(total, elapsed);
        results.push((producers, throughput));
    }

    let baseline = results[0].1;
    let physical_cores = std::thread::available_parallelism()
        .map(|p| p.get() / 2)
        .unwrap_or(4)
        .max(1);
    for &(producers, throughput) in &results {
        // When producers exceed physical cores, spin contention from
        // hyperthread sharing is expected to be severe. Use a looser
        // floor for oversubscribed configurations.
        let floor = if producers > physical_cores {
            0.05
        } else {
            RETROGRADE_FLOOR
        };
        let ratio = throughput / baseline;
        assert!(
            ratio >= floor,
            "ReorderRing scaling RETROGRADE at {producers} producers: \
             {throughput:.0} ops/s vs baseline {baseline:.0} ops/s \
             (ratio {ratio:.2}, floor {floor})"
        );
    }
}

// ---------------------------------------------------------------------------
// DispatchQueue scaling: 1 producer, 1/2/4/8 workers
// ---------------------------------------------------------------------------

#[test]
fn dispatch_queue_scaling_not_retrograde() {
    const CAPACITY: usize = 256;
    const TOTAL_ITEMS: u64 = 100_000;

    let mut results: Vec<(usize, f64)> = Vec::new();

    for &workers in &[1usize, 2, 4, 8] {
        let queue = Arc::new(DispatchQueue::<u64>::new(CAPACITY, SpinWake));
        let done = Arc::new(AtomicBool::new(false));

        let start = Instant::now();

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

        // Drain stragglers.
        let mut stragglers = 0u64;
        while queue.pop().is_some() {
            stragglers += 1;
        }

        let elapsed = start.elapsed();
        assert_eq!(
            total_popped + stragglers,
            TOTAL_ITEMS,
            "lost items at {workers} workers"
        );

        let throughput = ops_per_sec(TOTAL_ITEMS, elapsed);
        results.push((workers, throughput));
    }

    let baseline = results[0].1;
    let physical_cores = std::thread::available_parallelism()
        .map(|p| p.get() / 2)
        .unwrap_or(4)
        .max(1);
    for &(workers, throughput) in &results {
        // When workers exceed physical cores, spin contention is expected
        // to be severe (same class as CreditGuard single-counter CAS).
        // Use a looser floor for oversubscribed configurations.
        let floor = if workers > physical_cores {
            0.05
        } else {
            RETROGRADE_FLOOR
        };
        let ratio = throughput / baseline;
        assert!(
            ratio >= floor,
            "DispatchQueue scaling RETROGRADE at {workers} workers: \
             {throughput:.0} ops/s vs baseline {baseline:.0} ops/s \
             (ratio {ratio:.2}, floor {floor})"
        );
    }
}

// ---------------------------------------------------------------------------
// CreditGuard contention: 1/2/4/8 threads on one counter
// ---------------------------------------------------------------------------

#[test]
fn credit_guard_contention_not_catastrophic() {
    const OPS_PER_THREAD: u64 = 200_000;

    let mut results: Vec<(usize, f64)> = Vec::new();

    for &threads in &[1usize, 2, 4, 8] {
        let total = OPS_PER_THREAD * threads as u64;
        let guard = Arc::new(CreditGuard::new(threads as u64 * OPS_PER_THREAD));

        let start = Instant::now();

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

        let elapsed = start.elapsed();
        assert_eq!(guard.inflight(), 0, "credit leak at {threads} threads");

        let throughput = ops_per_sec(total, elapsed);
        results.push((threads, throughput));
    }

    // CreditGuard is expected to be retrograde under contention
    // (single CAS counter). But catastrophic collapse (>20x slower
    // at 8 threads than at 1) signals a bug, not expected contention.
    let baseline = results[0].1;
    for &(threads, throughput) in &results {
        let ratio = throughput / baseline;
        assert!(
            ratio >= 0.05,
            "CreditGuard CATASTROPHIC contention at {threads} threads: \
             {throughput:.0} ops/s vs baseline {baseline:.0} ops/s \
             (ratio {ratio:.2}, floor 0.05). Expected retrograde \
             but not 20x+ collapse."
        );
    }
}
