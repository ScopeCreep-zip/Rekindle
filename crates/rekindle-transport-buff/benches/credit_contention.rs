//! [`CreditGuard`] under N concurrent reservers.
//!
//! Measures try_reserve + release cycle throughput with 1, 2, 4, 8 threads
//! competing on a single counter.

use std::sync::Arc;
use std::thread;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rekindle_transport_buff::CreditGuard;

const OPS_PER_THREAD: u64 = 100_000;

fn bench_credit(c: &mut Criterion) {
    let mut group = c.benchmark_group("credit_contention");
    group.measurement_time(std::time::Duration::from_secs(30));
    group.sample_size(30);

    for &threads in &[1usize, 2, 4, 8] {
        let total = OPS_PER_THREAD * threads as u64;
        group.throughput(Throughput::Elements(total));

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{threads}_threads")),
            &threads,
            |b, &n_threads| {
                b.iter(|| {
                    // Ceiling high enough that reserves never fail from capacity.
                    let guard = Arc::new(CreditGuard::new(n_threads as u64 * OPS_PER_THREAD));

                    let handles: Vec<_> = (0..n_threads)
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

                    assert_eq!(guard.inflight(), 0);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_credit);
criterion_main!(benches);
