//! Scheduler primitive irreducible floors: rayon spawn round-trip.
//!
//! Report structure:
//! - rayon_spawn: line — function=spawn, param=workers (linear)
//!   Shows spawn+completion latency as a function of pool size.
//!
//! SamplingMode::Flat — spawn round-trip includes thread wakeup
//! and mpsc channel overhead, not linearly scalable.
//!
//! The mpsc channel is created per-iteration (not hoisted) because
//! rayon spawn() requires an owned closure — the tx must be moved
//! into the closure on every call. This matches the production
//! BulkSender pattern where each chunk spawns a fresh rayon task.

use std::time::Duration;

use criterion::{BenchmarkId, Criterion, SamplingMode, Throughput};
use rekindle_transport_ipc::calibrate::{OscId, Profile, CalibratedSession};

pub fn register(c: &mut Criterion, session: &mut CalibratedSession) {
    rayon_spawn(c, session);
}

fn rayon_spawn(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("rayon_spawn");
    group.throughput(Throughput::Elements(1));
    group.sampling_mode(SamplingMode::Flat);

    for &workers in &[1usize, 2, 4, 8] {
        let osc = format!("osc:lat/sched.spawn?variant=rayon&workers={workers}w&contention=none");
        let id = OscId::parse(&osc, Profile::L1Conditions)
            .unwrap_or_else(|e| panic!("malformed OSC: sched.spawn workers={workers}: {e}"));

        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        // Warm up all threads — wait_until_primed equivalent
        pool.install(|| {});
        std::thread::sleep(Duration::from_millis(10));

        // Raw numeric param for line chart X axis
        group.bench_function(BenchmarkId::new("spawn", format!("{workers}")), |b| {
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let (tx, rx) = std::sync::mpsc::channel();
                    pool.spawn(move || { let _ = tx.send(()); });
                    rx.recv().unwrap();
                }
                start.elapsed()
            })
        });

        let workers_str = format!("{workers}");
        session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
            Some(("rayon_spawn", "spawn", &workers_str)));
    }

    group.finish();
}
