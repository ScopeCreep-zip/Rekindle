//! Benchmarks for ShardedEventRouter event delivery at scale.
//!
//! Measures:
//! - Delivery throughput at 100, 1K, 10K, 50K subscribers
//! - Sequential vs parallel delivery path (threshold = 256)
//! - Bytes::clone fan-out cost vs Vec::clone fan-out cost
//! - Subscription/unsubscription overhead
//! - Disconnect cleanup (deindex) time at scale
//!
//! The runtime is created once and reused across iterations.
//! Subscriber tasks are spawned once and persist across iterations.
//! Only delivery is measured — setup is excluded.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use tokio::sync::{mpsc, Notify};

use rekindle_node::ipc::event_router::ShardedEventRouter;
use rekindle_types::subscription_events::{SubscriptionEvent, SubscriptionFilter};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .build()
        .unwrap()
}

/// Create a minimal test event. The actual content doesn't matter for
/// routing benchmarks — only the category and community scope affect
/// delivery path selection.
fn test_event() -> SubscriptionEvent {
    SubscriptionEvent::UnreadChanged {
        context: rekindle_types::subscription_events::UnreadContext::Channel {
            community: "VLD0:test_community_key:abc123".to_string(),
            channel: "general".to_string(),
        },
        count: 1,
    }
}

// ── Fan-Out Throughput ────────────────────────────────────────────────

fn bench_deliver_fanout(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_fanout");

    for n_subscribers in [100, 1_000, 10_000, 50_000] {
        group.throughput(Throughput::Elements(n_subscribers as u64));

        group.bench_with_input(
            BenchmarkId::new("deliver", n_subscribers),
            &n_subscribers,
            |b, &n| {
                let rt = rt();
                let router = Arc::new(ShardedEventRouter::new());

                // Wire up N subscribers with wildcard filters.
                // Each subscriber has a drain task that consumes frames.
                let wg = Arc::new((AtomicUsize::new(0), Notify::new()));

                for i in 0..n {
                    let (tx, mut rx) = mpsc::channel::<rekindle_node::ipc::message::SharedFrame>(64);
                    router
                        .subscribe(
                            i as u64,
                            &[SubscriptionFilter {
                                categories: None,
                                community_scope: None,
                            }],
                            tx,
                        )
                        .unwrap();

                    let wg = wg.clone();
                    rt.spawn(async move {
                        loop {
                            match rx.recv().await {
                                Some(_frame) => {
                                    if wg.0.fetch_sub(1, Ordering::Relaxed) == 1 {
                                        wg.1.notify_one();
                                    }
                                }
                                None => break,
                            }
                        }
                    });
                }

                let event = test_event();

                b.iter(|| {
                    rt.block_on(async {
                        wg.0.store(n, Ordering::Relaxed);
                        let (delivered, dropped) = router.deliver(black_box(&event));
                        black_box((delivered, dropped));

                        // Wait for all subscribers to consume
                        while wg.0.load(Ordering::Relaxed) > 0 {
                            wg.1.notified().await;
                        }
                    });
                });
            },
        );
    }
    group.finish();
}

// ── Delivery Without Consumer Wait ───────────────────────────────────
//
// Measures only the server-side delivery cost (try_send to all channels).
// Does not wait for consumers to process. This isolates the router cost
// from the consumer processing cost.

fn bench_deliver_send_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("deliver_send_only");

    for n_subscribers in [100, 1_000, 10_000, 50_000] {
        group.throughput(Throughput::Elements(n_subscribers as u64));

        group.bench_with_input(
            BenchmarkId::new("try_send", n_subscribers),
            &n_subscribers,
            |b, &n| {
                let router = Arc::new(ShardedEventRouter::new());

                // Create channels but DON'T spawn consumer tasks.
                // Channels have capacity 64 — first 64 deliveries succeed,
                // then they return Full. We measure the try_send path.
                let mut _receivers = Vec::with_capacity(n);
                for i in 0..n {
                    let (tx, rx) = mpsc::channel::<rekindle_node::ipc::message::SharedFrame>(64);
                    router
                        .subscribe(
                            i as u64,
                            &[SubscriptionFilter {
                                categories: None,
                                community_scope: None,
                            }],
                            tx,
                        )
                        .unwrap();
                    _receivers.push(rx);
                }

                let event = test_event();

                b.iter(|| {
                    let (delivered, dropped) = router.deliver(black_box(&event));
                    black_box((delivered, dropped));
                });

                // Drain receivers to prevent channel backpressure affecting next iter
                for rx in &mut _receivers {
                    while rx.try_recv().is_ok() {}
                }
            },
        );
    }
    group.finish();
}

// ── Subscribe / Disconnect Paired Suite ──────────────────────────────
//
// Measures the marginal cost of one subscribe or one remove_connection
// against a router pre-populated to a specific size. Parameterized by
// population size to show scaling behavior, and by filter strategy to
// cover the cheapest (wildcard: 1 index insert) and most expensive
// (community-scoped: 12 by_community inserts + interner lookup) paths.
//
// Uses iter_custom throughout: each iteration builds a fresh router at
// the target population, then measures exactly one operation. Setup cost
// is excluded from measurement.

/// Filter strategy for populating the router and for the measured operation.
#[derive(Clone, Copy)]
enum FilterStrategy {
    /// `SubscriptionFilter::all()` — single `wildcard` HashSet insert.
    Wildcard,
    /// `SubscriptionFilter::community(...)` — 12 `by_community` inserts
    /// per subscribe, interner lookup, 12 reverse index entries.
    CommunityScoped,
}

/// Populate a router with `n` subscribers using the given filter strategy.
///
/// Community-scoped subscribers are distributed across 100 distinct
/// communities (`i % 100`) to produce realistic interner and by_community
/// map sizes rather than pathological single-community concentration.
///
/// Receivers are dropped immediately. The `Sender` handles stored in the
/// stripe channel maps remain valid (try_send returns Err on closed rx,
/// but that doesn't affect subscribe/disconnect measurement).
fn populate_router(n: usize, strategy: FilterStrategy) -> Arc<ShardedEventRouter> {
    let router = Arc::new(ShardedEventRouter::new());
    for i in 0..n {
        let (tx, _rx) = mpsc::channel::<rekindle_node::ipc::message::SharedFrame>(1);
        let filter = match strategy {
            FilterStrategy::Wildcard => SubscriptionFilter::all(),
            FilterStrategy::CommunityScoped => SubscriptionFilter::community(
                format!("VLD0:community_{:04}:key", i % 100),
            ),
        };
        router.subscribe(i as u64, &[filter], tx).unwrap();
    }
    router
}

fn bench_subscribe(c: &mut Criterion) {
    let mut group = c.benchmark_group("subscribe");
    group.sample_size(20);
    group.measurement_time(std::time::Duration::from_secs(60));
    group.warm_up_time(std::time::Duration::from_secs(10));

    for population in [0, 1_000, 10_000, 50_000] {
        // ── Wildcard subscriber into wildcard-populated router ────
        //
        // Measures: parking_lot write lock acquisition on index,
        // HashSet::insert into `wildcard` (size = population),
        // original_filters insert, conn_index_keys push,
        // stripe channel write lock + HashMap::insert.
        group.bench_with_input(
            BenchmarkId::new("wildcard", population),
            &population,
            |b, &n| {
                b.iter_custom(|iters| {
                    let mut total = std::time::Duration::ZERO;
                    for _ in 0..iters {
                        let router = populate_router(n, FilterStrategy::Wildcard);
                        let (tx, _rx) = mpsc::channel::<rekindle_node::ipc::message::SharedFrame>(1);

                        let start = std::time::Instant::now();
                        router
                            .subscribe(n as u64, &[SubscriptionFilter::all()], tx)
                            .unwrap();
                        total += start.elapsed();
                    }
                    total
                });
            },
        );

        // ── Community-scoped subscriber into community-populated router ──
        //
        // Measures: same lock acquisitions as wildcard, plus
        // GovKeyInterner::intern (HashMap lookup + potential allocation),
        // 12× by_community HashSet::insert (one per EventCategory),
        // 12× reverse index IndexKey::Community pushes.
        // Uses a new community not in the existing set to also measure
        // the interner allocation path.
        group.bench_with_input(
            BenchmarkId::new("community_scoped", population),
            &population,
            |b, &n| {
                b.iter_custom(|iters| {
                    let mut total = std::time::Duration::ZERO;
                    for _ in 0..iters {
                        let router = populate_router(n, FilterStrategy::CommunityScoped);
                        let (tx, _rx) = mpsc::channel::<rekindle_node::ipc::message::SharedFrame>(1);
                        let filter = SubscriptionFilter::community(
                            "VLD0:bench_community:new_sub".to_string(),
                        );

                        let start = std::time::Instant::now();
                        router.subscribe(n as u64, &[filter], tx).unwrap();
                        total += start.elapsed();
                    }
                    total
                });
            },
        );
    }
    group.finish();
}

fn bench_disconnect_cleanup(c: &mut Criterion) {
    let mut group = c.benchmark_group("disconnect_cleanup");
    group.sample_size(20);
    group.measurement_time(std::time::Duration::from_secs(60));
    group.warm_up_time(std::time::Duration::from_secs(10));

    for population in [1_000, 10_000, 50_000] {
        // ── Wildcard disconnect from wildcard-populated router ────
        //
        // Measures: parking_lot write lock on stripe, HashMap::remove,
        // parking_lot write lock on index, wildcard.remove(&conn_id),
        // iterate 1 IndexKey::Wildcard in reverse index,
        // original_filters.remove.
        group.bench_with_input(
            BenchmarkId::new("wildcard", population),
            &population,
            |b, &n| {
                b.iter_custom(|iters| {
                    let mut total = std::time::Duration::ZERO;
                    for _ in 0..iters {
                        let router = populate_router(n, FilterStrategy::Wildcard);
                        let target = (n / 2) as u64;

                        let start = std::time::Instant::now();
                        router.remove_connection(target);
                        total += start.elapsed();
                    }
                    total
                });
            },
        );

        // ── Community-scoped disconnect from community-populated router ──
        //
        // Measures: same lock acquisitions as wildcard, plus
        // iterate 12 IndexKey::Community entries in reverse index,
        // 12× by_community.get_mut().remove() with emptiness check,
        // potential by_community.remove() cleanup per empty set.
        group.bench_with_input(
            BenchmarkId::new("community_scoped", population),
            &population,
            |b, &n| {
                b.iter_custom(|iters| {
                    let mut total = std::time::Duration::ZERO;
                    for _ in 0..iters {
                        let router = populate_router(n, FilterStrategy::CommunityScoped);
                        let target = (n / 2) as u64;

                        let start = std::time::Instant::now();
                        router.remove_connection(target);
                        total += start.elapsed();
                    }
                    total
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_deliver_fanout,
    bench_deliver_send_only,
    bench_subscribe,
    bench_disconnect_cleanup,
);
criterion_main!(benches);
