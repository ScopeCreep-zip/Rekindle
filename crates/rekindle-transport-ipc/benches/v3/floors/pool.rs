//! Pool primitive irreducible floors: acquire/release/reclaim cycle.
//!
//! Report structure:
//! - pool: line — function=acquire, param=threads (linear)
//!   Shows contention scaling: 1 (uncontended) / 2 / 4 / 8 threads.
//!
//! Uses the transport's BufferPool to measure the actual pool primitive
//! cost, not a synthetic proxy. Contended variants use bench_contended
//! from floors/mod.rs.
//!
//! SamplingMode::Flat for contended variants (thread lifecycle inside
//! bench_function). Auto for single-threaded (tight loop).

use criterion::{BenchmarkId, Criterion, SamplingMode, Throughput};
use rekindle_transport_ipc::calibrate::{OscId, Profile, CalibratedSession};
use rekindle_transport_ipc::v3::bulk::pool::BufferPool;

use super::bench_contended;

pub fn register(c: &mut Criterion, session: &mut CalibratedSession) {
    pool_cycle(c, session);
}

fn pool_cycle(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("pool_contention");
    group.throughput(Throughput::Elements(1));

    // Single-threaded acquire/release/reclaim — uncontended baseline
    {
        let id = OscId::parse(
            "osc:lat/pool.acquire?contention=none",
            Profile::L1Conditions,
        ).expect("malformed OSC: pool.acquire uncontended");

        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
        } else {
            let pool = BufferPool::with_capacity(64, 1024);
            // Raw numeric param "1" for line chart — uncontended is 1 thread
            group.bench_function(BenchmarkId::new("acquire", "1"), |b| {
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        if let Some(slab) = pool.try_acquire() {
                            pool.release(slab);
                            pool.reclaim();
                        }
                    }
                    start.elapsed()
                })
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("pool_contention", "acquire", "1")));
        }
    }

    // Contended acquire/release — shows scaling curve
    group.sampling_mode(SamplingMode::Flat);
    for &threads in &[2usize, 4, 8] {
        let osc = format!("osc:lat/pool.acquire?contention={threads}t");
        let id = OscId::parse(&osc, Profile::L1Conditions)
            .unwrap_or_else(|e| panic!("malformed OSC: pool.acquire contention={threads}t: {e}"));

        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        let pool = BufferPool::with_capacity(32, 1024);
        let p = pool.clone();

        // Raw numeric param for line chart X axis
        bench_contended(&mut group, &format!("acquire/{threads}"), threads, move |n| {
            for _ in 0..n {
                if let Some(slab) = p.try_acquire() {
                    p.release(slab);
                    p.reclaim();
                }
            }
        });

        // bench_contended uses bench_function(name) not BenchmarkId::new(fn, param),
        // so criterion stores it as function_id with no value_str.
        // Directory: pool_contention/acquire_{threads}/new/estimates.json
        let fn_name = format!("acquire/{threads}");
        session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
            Some(("pool_contention", &fn_name, "")));
    }

    group.finish();
}
