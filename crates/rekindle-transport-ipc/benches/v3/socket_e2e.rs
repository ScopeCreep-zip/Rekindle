//! v3 socket benchmarks: real AF_UNIX, multi-thread tokio, full pipeline.
//!
//! Every benchmark runs over a real Unix socket with real Noise IK handshake,
//! real EMAC+HeaderMAC+AEAD on every frame, real rayon parallel encryption.
//!
//! Each measurement is identified by an `osc:` identifier, recorded with harness
//! coordinates for backfill, and emitted as structured JSONL via `finalize()`.
//!
//! No assertions inside b.iter() closures. No duplicated fixture code.
//! All connection lifecycle via fixture::IpcFixture.

use std::time::Duration;
use criterion::{BenchmarkId, Criterion, Throughput};

use rekindle_transport_ipc::calibrate::{
    CalibratedSession, OscId, Profile,
    init_tracing, finalize, fmt_mag,
};
use rekindle_transport_ipc::fixture::{
    IpcFixture, IpcFixtureConfig, payload, phys_cores,
    BENCH_TIMEOUT,
};
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

fn osc_record(
    session: &mut CalibratedSession,
    osc_str: &str,
    group: &str,
    function: &str,
    param: &str,
    sample_count: u32,
) {
    let id = OscId::parse(osc_str, Profile::L0Core)
        .unwrap_or_else(|e| panic!("malformed OSC id '{osc_str}': {e}"));
    session.record_with_coords(
        &id, 0.0, 0.0, 0.0, sample_count, "criterion.pending_backfill",
        Some((group, function, param)),
    );
}

// ── Storage CRUD ────────────────────────────────────────────────

fn storage_crud_latency(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut group = c.benchmark_group("storage_crud");
    for &(size, label, osc_op) in &[
        (80, "resolve_friend_name_req", "storage.resolve"),
        (140, "resolve_friend_name_reply", "storage.resolve_reply"),
        (136, "delete_key_req", "storage.delete"),
        (200, "store_friend_name_req", "storage.store"),
        (4, "count_keys_req", "storage.count"),
        (12, "count_keys_reply", "storage.count_reply"),
    ] {
        let data = payload(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &data, |b, data| {
            b.iter(|| f.block_on(f.send_request(data, BENCH_TIMEOUT)).unwrap())
        });
        osc_record(session, &format!("osc:lat/{osc_op}?size={size}"), "storage_crud", label, "", 20);
    }
    group.finish();
}

fn storage_batch(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut group = c.benchmark_group("storage_batch");
    for &(size, label) in &[(10 * 80, "10_ops"), (50 * 80, "50_ops"), (200 * 80, "200_ops")] {
        let data = payload(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &data, |b, data| {
            b.iter(|| f.block_on(f.send_request(data, BENCH_TIMEOUT)).unwrap())
        });
        let mag = fmt_mag(size);
        osc_record(session, &format!("osc:lat/storage.batch?size={mag}"), "storage_batch", label, "", 20);
    }
    group.finish();
}

// ── Chat ────────────────────────────────────────────────────────

fn chat_sustained_requests(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let msg = payload(256);
    let mut group = c.benchmark_group("chat_sustained");
    group.throughput(Throughput::Elements(100));
    group.bench_function("100_sequential_requests", |b| {
        b.iter(|| f.block_on(async {
            for _ in 0..100 { f.send_request(&msg, BENCH_TIMEOUT).await.unwrap(); }
        }))
    });
    osc_record(session, "osc:ops/ipc.request?size=256&contention=none", "chat_sustained", "100_sequential_requests", "", 20);
    group.finish();
}

fn chat_attachment_throughput(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut group = c.benchmark_group("attachment_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    {
        let data = payload(1024);
        group.throughput(Throughput::Bytes(1024));
        group.bench_with_input(BenchmarkId::from_parameter("1KiB_tiny_image"), &data, |b, data| {
            b.iter(|| f.block_on(f.send_request(data, BENCH_TIMEOUT)).unwrap())
        });
        osc_record(session, "osc:bw/ipc.request?size=1k", "attachment_throughput", "1KiB_tiny_image", "", 10);
    }
    for &(size, label) in &[
        (100 * 1024, "100KiB_photo_thumb"),
        (1024 * 1024, "1MiB_document"),
        (10 * 1024 * 1024, "10MiB_photo"),
        (64 * 1024 * 1024, "64MiB_video"),
    ] {
        let data = payload(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &data, |b, data| {
            b.iter(|| f.block_on(f.send_bulk(0, data, BENCH_TIMEOUT)).unwrap())
        });
        let mag = fmt_mag(size);
        osc_record(session, &format!("osc:bw/ipc.bulk?size={mag}"), "attachment_throughput", label, "", 10);
    }
    group.finish();
}

fn chat_mixed_workload(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let attachment = payload(10 * 1024 * 1024);
    let msg = payload(256);
    let mut group = c.benchmark_group("mixed_workload");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    group.throughput(Throughput::Bytes((attachment.len() + msg.len() * 50) as u64));
    group.bench_function("10MiB_upload_plus_50_messages", |b| {
        b.iter(|| f.block_on(async {
            let bulk_fut = f.send_bulk(0, &attachment, BENCH_TIMEOUT);
            let msg_fut = async {
                for _ in 0..50 { f.send_request(&msg, BENCH_TIMEOUT).await.unwrap(); }
            };
            let (bulk_res, _) = tokio::join!(bulk_fut, msg_fut);
            bulk_res.unwrap();
        }))
    });
    osc_record(session, "osc:bw/ipc.bulk+ipc.request?size=10m&contention=mixed", "mixed_workload", "10MiB_upload_plus_50_messages", "", 10);
    group.finish();
}

// ── Escalation boundary ─────────────────────────────────────────

fn stream_escalation_boundary(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut group = c.benchmark_group("escalation_boundary");
    for &(size, label) in &[
        (32 * 1024, "32KiB_inline"), (63 * 1024, "63KiB_inline"),
        (65 * 1024, "65KiB_bulk"), (128 * 1024, "128KiB_bulk"), (256 * 1024, "256KiB_bulk"),
    ] {
        let data = payload(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &data, |b, data| {
            b.iter(|| f.block_on(f.send_bulk(0, data, BENCH_TIMEOUT)).unwrap())
        });
        let mag = fmt_mag(size);
        osc_record(session, &format!("osc:bw/ipc.bulk?size={mag}&variant=escalation"), "escalation_boundary", label, "", 20);
    }
    group.finish();
}

// ── Cold start ──────────────────────────────────────────────────

fn cold_start_latency(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut group = c.benchmark_group("cold_start");
    group.sample_size(10);
    group.bench_function("connect_first_request_shutdown", |b| {
        b.iter(|| f.block_on(async {
            let extra = f.connect_additional_client().await;
            extra.send_request(b"first", BENCH_TIMEOUT).await.unwrap();
            extra.shutdown().await;
        }))
    });
    osc_record(session, "osc:lat/ipc.handshake+ipc.request?variant=cold_start", "cold_start", "connect_first_request_shutdown", "", 10);
    group.finish();
}

// ── Concurrent bulk ─────────────────────────────────────────────

fn concurrent_bulk(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let chunk = payload(16 * 1024 * 1024);
    let mut group = c.benchmark_group("concurrent_bulk");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    let cores = phys_cores() as u8;
    let counts: Vec<u8> = vec![2, cores].into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    for &n in &counts {
        group.throughput(Throughput::Bytes(chunk.len() as u64 * n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(format!("{n}_streams")), &n, |b, &count| {
            b.iter(|| f.block_on(async {
                let futs: Vec<_> = (0..count).map(|sid| f.send_bulk(sid, &chunk, BENCH_TIMEOUT)).collect();
                futures::future::join_all(futs).await
            }))
        });
        osc_record(session, &format!("osc:bw/ipc.bulk?size=16m&contention={n}s"), "concurrent_bulk", &format!("{n}_streams"), "", 10);
    }
    group.finish();
}

// ── Notify throughput ───────────────────────────────────────────

fn notify_throughput(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let msg = payload(128);
    let mut group = c.benchmark_group("notify_throughput");
    group.throughput(Throughput::Elements(1000));
    group.bench_function("1000_fire_and_forget", |b| {
        b.iter(|| f.block_on(async {
            for _ in 0..1000 { let _ = f.send_notify(&msg).await; }
        }))
    });
    osc_record(session, "osc:ops/ipc.notify?size=128&contention=none", "notify_throughput", "1000_fire_and_forget", "", 20);
    group.finish();
}

// ── Multi-client ────────────────────────────────────────────────

fn multi_client_throughput(c: &mut Criterion, session: &mut CalibratedSession) {
    let cores = phys_cores();
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut extra_clients = Vec::new();
    for _ in 1..cores {
        extra_clients.push(f.block_on(f.connect_additional_client()));
    }

    let msg = payload(256);
    let mut group = c.benchmark_group("multi_client");
    let client_counts: Vec<usize> = vec![1, 2, cores].into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    for &n in &client_counts {
        group.throughput(Throughput::Elements(n as u64 * 10));
        group.bench_with_input(BenchmarkId::from_parameter(format!("{n}_clients")), &n, |b, &num| {
            b.iter(|| f.block_on(async {
                // First client is f.client, rest from extra_clients
                let mut futs: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>>> = Vec::new();
                if num >= 1 {
                    futs.push(Box::pin(async {
                        for _ in 0..10 { f.send_request(&msg, BENCH_TIMEOUT).await.unwrap(); }
                    }));
                }
                for client in extra_clients.iter().take(num.saturating_sub(1)) {
                    futs.push(Box::pin(async {
                        for _ in 0..10 { client.send_request(&msg, BENCH_TIMEOUT).await.unwrap(); }
                    }));
                }
                futures::future::join_all(futs).await;
            }))
        });
        osc_record(session, &format!("osc:ops/ipc.request?size=256&contention={n}c"), "multi_client", &format!("{n}_clients"), "", 20);
    }
    group.finish();
}

// ── Sustained bulk throughput ───────────────────────────────────

fn sustained_bulk_throughput(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let chunk = payload(16 * 1024 * 1024);
    let mut group = c.benchmark_group("sustained_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    group.throughput(Throughput::Bytes(chunk.len() as u64));
    group.bench_function("16MiB_sustained", |b| {
        b.iter(|| f.block_on(f.send_bulk(0, &chunk, BENCH_TIMEOUT)).unwrap())
    });
    osc_record(session, "osc:bw/ipc.bulk?size=16m&state=sustained", "sustained_throughput", "16MiB_sustained", "", 10);
    group.finish();
}

// ── Connection density ──────────────────────────────────────────

fn connection_density(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut group = c.benchmark_group("connection_density");
    group.sample_size(10);
    group.bench_function("100_idle_connections", |b| {
        b.iter(|| f.block_on(async {
            let mut clients = Vec::new();
            for _ in 0..100 {
                clients.push(f.connect_additional_client().await);
            }
            std::hint::black_box(clients.len());
            for c in clients { c.shutdown().await; }
        }))
    });
    osc_record(session, "osc:cost/ipc.connection?contention=100c&res=rss&variant=idle", "connection_density", "100_idle_connections", "", 10);
    group.finish();
}

// ── Memory stability (throughput only — RSS assertion in tests) ─

fn memory_stability(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let data = payload(64 * 1024);
    let mut group = c.benchmark_group("memory_stability");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    group.bench_function("500_transfers_sustained", |b| {
        b.iter(|| f.block_on(async {
            for _ in 0..500 { f.send_bulk(0, &data, BENCH_TIMEOUT).await.unwrap(); }
        }))
    });
    osc_record(session, "osc:cost/ipc.bulk?res=rss&size=64k&state=sustained&variant=leak_check", "memory_stability", "500_transfers_sustained", "", 10);
    group.finish();
}

// ── Reconnect under load ────────────────────────────────────────

fn reconnect_under_load(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let msg = payload(128);
    let mut group = c.benchmark_group("reconnect_under_load");
    group.sample_size(10);
    group.bench_function("connect_request_disconnect_while_busy", |b| {
        b.iter(|| f.block_on(async {
            f.send_request(&msg, BENCH_TIMEOUT).await.unwrap();
            let ephemeral = f.connect_additional_client().await;
            ephemeral.send_request(&msg, BENCH_TIMEOUT).await.unwrap();
            ephemeral.shutdown().await;
        }))
    });
    osc_record(session, "osc:lat/ipc.handshake+ipc.request?variant=reconnect&contention=busy", "reconnect_under_load", "connect_request_disconnect_while_busy", "", 10);
    group.finish();
}

// ── Server→Client bulk ──────────────────────────────────────────

fn server_to_client_bulk(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut group = c.benchmark_group("server_to_client_bulk");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    for &(size, label) in &[(1024, "1KiB"), (64 * 1024, "64KiB"), (1024 * 1024, "1MiB")] {
        let data = payload(size);
        let sender = f.bulk_sender();
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &data, |b, data| {
            b.iter(|| f.block_on(async {
                sender.send(0, data, Clearance::Internal).await.unwrap();
                f.recv_bulk().await.expect("must receive server-sent bulk");
            }))
        });
        let mag = fmt_mag(size);
        osc_record(session, &format!("osc:bw/ipc.bulk?size={mag}&variant=server_to_client"), "server_to_client_bulk", label, "", 10);
    }
    group.finish();
}

// ── Bidirectional datagram ──────────────────────────────────────

fn bidirectional_datagram(c: &mut Criterion, session: &mut CalibratedSession) {
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let event = payload(256);
    let tx = f.outbound_tx();
    let mut group = c.benchmark_group("bidirectional_datagram");
    group.throughput(Throughput::Bytes(256));
    group.bench_function("server_notify_client_recv", |b| {
        b.iter(|| f.block_on(async {
            let notify = rekindle_transport_ipc::v3::codec::datagram::notify::encode(
                &rekindle_transport_ipc::v3::codec::datagram::notify::DatagramNotifyPayload {
                    message_id: uuid::Uuid::now_v7(),
                    sender_clearance: Clearance::Internal,
                    application_payload: event.clone(),
                },
            );
            tx.send(rekindle_transport_ipc::v3::context::OutboundFrame::Datagram {
                kind: rekindle_transport_ipc::v3::wire::frame_kind::DatagramKind::Notify,
                payload: notify,
            }).await.expect("server outbound send");
            f.recv().await.expect("must receive server-sent notify");
        }))
    });
    osc_record(session, "osc:lat/ipc.notify?size=256&variant=server_to_client", "bidirectional_datagram", "server_notify_client_recv", "", 20);
    group.finish();
}

// ── Pubsub fanout ───────────────────────────────────────────────

fn pubsub_fanout(c: &mut Criterion, session: &mut CalibratedSession) {
    let cores = phys_cores();
    let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
    let mut extra = Vec::new();
    for _ in 1..cores {
        extra.push(f.block_on(f.connect_additional_client()));
    }

    let event = payload(256);
    let mut group = c.benchmark_group("pubsub_fanout");

    let n_pub = 1 + extra.len(); // f.client + extras
    group.throughput(Throughput::Elements(n_pub as u64));
    group.bench_function(&format!("{n_pub}_publishers_1_event_each"), |b| {
        b.iter(|| f.block_on(async {
            let _ = f.send_notify(&event).await;
            for c in &extra { let _ = c.send_notify(&event).await; }
        }))
    });
    osc_record(session, &format!("osc:ops/ipc.notify?size=256&contention={n_pub}c&variant=fanout"), "pubsub_fanout", &format!("{n_pub}_publishers_1_event_each"), "", 20);

    group.throughput(Throughput::Elements(100));
    group.bench_function("1_publisher_100_events", |b| {
        b.iter(|| f.block_on(async {
            for _ in 0..100 { let _ = f.send_notify(&event).await; }
        }))
    });
    osc_record(session, "osc:ops/ipc.notify?size=256&contention=none&variant=burst", "pubsub_fanout", "1_publisher_100_events", "", 20);
    group.finish();
}

// ── Rotation ────────────────────────────────────────────────────

fn rotation_latency(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("rotation");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    // Each rotation bench gets a fresh connection to avoid accumulated
    // state from prior bench functions (epoch drift over 100K+ rotations).
    {
        let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
        group.bench_function("rotate_idle", |b| {
            b.iter(|| f.block_on(async {
                f.rotate_keys(BENCH_TIMEOUT).await.unwrap();
                f.send_request(b"sync", BENCH_TIMEOUT).await.unwrap();
            }))
        });
        osc_record(session, "osc:lat/ipc.rotate?contention=none&variant=idle", "rotation", "rotate_idle", "", 10);
    }

    {
        let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
        group.bench_function("rotate_then_request", |b| {
            b.iter(|| f.block_on(async {
                f.rotate_keys(BENCH_TIMEOUT).await.unwrap();
                f.send_request(b"post-rotation", BENCH_TIMEOUT).await.unwrap();
            }))
        });
        osc_record(session, "osc:lat/ipc.rotate+ipc.request?contention=none", "rotation", "rotate_then_request", "", 10);
    }

    {
        let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
        let bulk_data = payload(64 * 1024);
        group.throughput(Throughput::Bytes(64 * 1024));
        group.bench_function("rotate_then_64KiB_bulk", |b| {
            b.iter(|| f.block_on(async {
                f.rotate_keys(BENCH_TIMEOUT).await.unwrap();
                f.send_bulk(0, &bulk_data, BENCH_TIMEOUT).await.unwrap();
            }))
        });
        osc_record(session, "osc:lat/ipc.rotate+ipc.bulk?size=64k&contention=none", "rotation", "rotate_then_64KiB_bulk", "", 10);
    }
    group.finish();
}

fn rotation_resilience(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("rotation_resilience");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    {
        let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
        group.bench_function("10_sequential_rotations", |b| {
            b.iter(|| f.block_on(async {
                for i in 0..10 {
                    f.rotate_keys(BENCH_TIMEOUT).await
                        .unwrap_or_else(|e| panic!("rotation {i} failed: {e:?}"));
                    f.send_request(b"alive", BENCH_TIMEOUT).await
                        .unwrap_or_else(|e| panic!("request after rotation {i} failed: {e:?}"));
                }
            }))
        });
        osc_record(session, "osc:lat/ipc.rotate?contention=none&variant=sequential_10x", "rotation_resilience", "10_sequential_rotations", "", 10);
    }

    {
        let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
        group.bench_function("rotate_during_requests", |b| {
            b.iter(|| f.block_on(async {
                for _ in 0..20 { f.send_request(b"pre", BENCH_TIMEOUT).await.unwrap(); }
                f.rotate_keys(BENCH_TIMEOUT).await.unwrap();
                for _ in 0..20 { f.send_request(b"post", BENCH_TIMEOUT).await.unwrap(); }
            }))
        });
        osc_record(session, "osc:lat/ipc.rotate?contention=requests&variant=mid_stream", "rotation_resilience", "rotate_during_requests", "", 10);
    }

    {
        let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
        let bulk_1m = payload(1024 * 1024);
        group.throughput(Throughput::Bytes(1024 * 1024));
        group.bench_function("rotate_then_1MiB_bulk", |b| {
            b.iter(|| f.block_on(async {
                f.rotate_keys(BENCH_TIMEOUT).await.unwrap();
                f.send_bulk(0, &bulk_1m, BENCH_TIMEOUT).await.unwrap();
            }))
        });
        osc_record(session, "osc:bw/ipc.rotate+ipc.bulk?size=1m&contention=none", "rotation_resilience", "rotate_then_1MiB_bulk", "", 10);
    }

    {
        let f = IpcFixture::connect_blocking(IpcFixtureConfig::for_bench());
        let bulk_1m = payload(1024 * 1024);
        group.throughput(Throughput::Bytes(1024 * 1024));
        group.bench_function("double_rotate_then_bulk", |b| {
            b.iter(|| f.block_on(async {
                f.rotate_keys(BENCH_TIMEOUT).await.unwrap();
                f.rotate_keys(BENCH_TIMEOUT).await.unwrap();
                f.send_bulk(0, &bulk_1m, BENCH_TIMEOUT).await.unwrap();
            }))
        });
        osc_record(session, "osc:bw/ipc.rotate+ipc.bulk?size=1m&contention=none&epoch=double", "rotation_resilience", "double_rotate_then_bulk", "", 10);
    }
    group.finish();
}

// ── Main ────────────────────────────────────────────────────────

fn main() {
    init_tracing();

    let mut criterion = Criterion::default()
        .measurement_time(Duration::from_secs(30))
        .sample_size(20)
        .nresamples(100_000)
        .configure_from_args();

    let mut session = CalibratedSession::new(Profile::L0Core);

    storage_crud_latency(&mut criterion, &mut session);
    storage_batch(&mut criterion, &mut session);
    chat_sustained_requests(&mut criterion, &mut session);
    chat_attachment_throughput(&mut criterion, &mut session);
    chat_mixed_workload(&mut criterion, &mut session);
    stream_escalation_boundary(&mut criterion, &mut session);
    cold_start_latency(&mut criterion, &mut session);
    concurrent_bulk(&mut criterion, &mut session);
    notify_throughput(&mut criterion, &mut session);
    multi_client_throughput(&mut criterion, &mut session);
    sustained_bulk_throughput(&mut criterion, &mut session);
    connection_density(&mut criterion, &mut session);
    memory_stability(&mut criterion, &mut session);
    reconnect_under_load(&mut criterion, &mut session);
    server_to_client_bulk(&mut criterion, &mut session);
    bidirectional_datagram(&mut criterion, &mut session);
    pubsub_fanout(&mut criterion, &mut session);
    rotation_latency(&mut criterion, &mut session);
    rotation_resilience(&mut criterion, &mut session);

    finalize(&mut session, &mut criterion, "e2e");
}
