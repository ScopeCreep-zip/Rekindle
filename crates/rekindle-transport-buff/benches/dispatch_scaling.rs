//! MPMC scaling for [`DispatchQueue`].
//!
//! Measures push/pop throughput with 1 producer and 1, 2, 4, 8 consumer
//! (worker) threads.

use std::sync::Arc;
use std::thread;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rekindle_transport_buff::{DispatchQueue, SpinWake};

const CAPACITY: usize = 256;
const TOTAL_ITEMS: u64 = 50_000;

fn bench_dispatch(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch_scaling");
    group.measurement_time(std::time::Duration::from_secs(30));
    group.sample_size(30);

    for &workers in &[1usize, 2, 4, 8] {
        group.throughput(Throughput::Elements(TOTAL_ITEMS));

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{workers}_workers")),
            &workers,
            |b, &n_workers| {
                b.iter(|| {
                    let queue = Arc::new(DispatchQueue::<u64, SpinWake>::new(CAPACITY, SpinWake));
                    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));

                    // Workers: pop until done signal, then drain remaining.
                    let worker_handles: Vec<_> = (0..n_workers)
                        .map(|_| {
                            let q = queue.clone();
                            let d = done.clone();
                            thread::spawn(move || {
                                let mut count = 0u64;
                                loop {
                                    if let Some(_) = q.pop() {
                                        count += 1;
                                    } else if d.load(std::sync::atomic::Ordering::Acquire) {
                                        // Drain any stragglers after done signal.
                                        while let Some(_) = q.pop() {
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

                    // Producer: push all items.
                    for seq in 0..TOTAL_ITEMS {
                        while queue.try_push(seq, seq).is_err() {
                            std::hint::spin_loop();
                        }
                    }

                    done.store(true, std::sync::atomic::Ordering::Release);

                    let total_popped: u64 = worker_handles
                        .into_iter()
                        .map(|h| h.join().unwrap())
                        .sum();

                    // Workers drained after done — stragglers may remain
                    // if multiple workers exit concurrently. Drain the queue.
                    let mut stragglers = 0u64;
                    while let Some(_) = queue.pop() {
                        stragglers += 1;
                    }

                    assert_eq!(total_popped + stragglers, TOTAL_ITEMS);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_dispatch);
criterion_main!(benches);
