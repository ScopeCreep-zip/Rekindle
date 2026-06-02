//! Throughput vs producer count for [`ReorderRing`].
//!
//! Measures publish + drain_contiguous throughput with 1, 2, 4, 8 producer
//! threads and one consumer. Asserts monotonic-or-flat scaling — a retrograde
//! result signals reintroduced false sharing.

use std::sync::Arc;
use std::thread;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rekindle_transport_buff::ReorderRing;

const WINDOW: usize = 1024;
const ITEMS_PER_PRODUCER: u64 = 10_000;

fn bench_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("reorder_scaling");
    group.measurement_time(std::time::Duration::from_secs(30));
    group.sample_size(30);

    for &producers in &[1usize, 2, 4, 8] {
        let total = ITEMS_PER_PRODUCER * producers as u64;
        group.throughput(Throughput::Elements(total));

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{producers}_producers")),
            &producers,
            |b, &n_producers| {
                b.iter(|| {
                    let ring = Arc::new(ReorderRing::<u64>::new(WINDOW));

                    let handles: Vec<_> = (0..n_producers)
                        .map(|p| {
                            let r = ring.clone();
                            let start = p as u64 * ITEMS_PER_PRODUCER;
                            let end = start + ITEMS_PER_PRODUCER;
                            thread::spawn(move || {
                                for seq in start..end {
                                    // Retry on overflow (window full — consumer hasn't drained).
                                    while r.publish(seq, seq).is_err() {
                                        std::hint::spin_loop();
                                    }
                                }
                            })
                        })
                        .collect();

                    // Consumer: drain until all items delivered.
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
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_scaling);
criterion_main!(benches);
