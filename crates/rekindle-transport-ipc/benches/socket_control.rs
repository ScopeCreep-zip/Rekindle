//! Socket-level control plane benchmarks.
//!
//! Measures connection lifecycle, frame routing latency, and event fan-out
//! over real Unix sockets with real Noise encryption.
//!
//! Run: cargo bench -p rekindle-transport-ipc --bench socket_control

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion, Throughput, BenchmarkId};

use rekindle_transport_ipc::client::IpcClient;
use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::envelope::SharedFrame;
use rekindle_transport_ipc::noise::keys::generate_keypair;
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::ConnectionPhase;

fn sock_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-bench-{}-{}.sock", label, std::process::id()
    ))
}

// ---- Benchmark: connection establishment rate (connects/sec) ----

fn bench_connection_rate(c: &mut Criterion) {
    struct NullRouter;
    impl FrameRouter for NullRouter {
        fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("conn-rate");
    let _ = std::fs::remove_file(&path);

    let pub_key = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pk: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), NullRouter, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;
        pk
    });

    let mut group = c.benchmark_group("connection_lifecycle");
    group.measurement_time(Duration::from_secs(30));
    group.sample_size(50);

    group.bench_function("connect_send_shutdown", |b| {
        b.iter(|| {
            rt.block_on(async {
                let ckp = generate_keypair().unwrap();
                let client = IpcClient::connect(
                    uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
                ).await.unwrap();
                client.send_frame(b"ping", Duration::from_secs(5)).await;
                client.shutdown().await;
            });
        });
    });

    group.bench_function("connect_shutdown", |b| {
        b.iter(|| {
            rt.block_on(async {
                let ckp = generate_keypair().unwrap();
                let client = IpcClient::connect(
                    uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
                ).await.unwrap();
                client.shutdown().await;
            });
        });
    });

    group.finish();
    let _ = std::fs::remove_file(&path);
}

// ---- Benchmark: event fan-out (1 trigger → N clients receive) ----

fn bench_event_fanout(c: &mut Criterion) {
    use tokio::sync::mpsc;

    /// Router that broadcasts to all connected clients via event_tx.
    struct BroadcastRouter;
    impl FrameRouter for BroadcastRouter {
        fn route_frame(&self, state: &ServerState, _conn_id: u64, payload: Bytes) {
            let event = SharedFrame::from_bytes(&payload);
            for entry in state.connections.iter() {
                let _ = entry.event_tx.try_send(event.clone());
            }
        }
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("fanout");
    let _ = std::fs::remove_file(&path);

    // Setup: connect 10 clients, split each into (Arc<IpcClient>, Receiver).
    // The Arc handles are used for triggering, the Receivers for verifying delivery.
    let (trigger, _keep_alive, mut receivers): (Arc<IpcClient>, Vec<Arc<IpcClient>>, Vec<mpsc::Receiver<bytes::Bytes>>) = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pk: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), BroadcastRouter, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let mut send_handles = Vec::new();
        let mut recv_handles = Vec::new();

        for _ in 0..10 {
            let ckp = generate_keypair().unwrap();
            let c = IpcClient::connect(
                uuid::Uuid::now_v7(), &path, &pk, ckp.as_inner(), &IpcConfig::default(), None,
            ).await.unwrap();
            let (send_handle, recv_handle) = c.into_split();
            send_handles.push(send_handle);
            recv_handles.push(recv_handle);
        }

        // Warmup: trigger one broadcast and drain all receivers.
        send_handles[0].send_frame(b"warmup", Duration::from_secs(5)).await;
        for rx in &mut recv_handles {
            let _ = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        }

        let trigger = send_handles[0].clone();
        (trigger, send_handles, recv_handles)
    });

    let mut group = c.benchmark_group("event_fanout");
    group.measurement_time(Duration::from_secs(15));

    // Measure full fan-out: trigger 1 frame → all N clients receive.
    // The benchmark loop sends the trigger AND drains all N receivers.
    group.throughput(Throughput::Elements(10));
    group.bench_function("broadcast_10_clients", |b| {
        b.iter(|| {
            rt.block_on(async {
                trigger.send_frame(b"event", Duration::from_secs(5)).await;
                for rx in receivers.iter_mut() {
                    tokio::time::timeout(Duration::from_secs(5), rx.recv())
                        .await.expect("event delivery timed out")
                        .expect("receiver channel closed");
                }
            });
        });
    });
    group.finish();

    drop(trigger);
    drop(_keep_alive);
    let _ = std::fs::remove_file(&path);
}

// ---- Benchmark: control frame throughput at varying sizes ----

fn bench_control_frame_throughput(c: &mut Criterion) {
    struct SinkRouter;
    impl FrameRouter for SinkRouter {
        fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("ctrl-tp");
    let _ = std::fs::remove_file(&path);

    let client = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), SinkRouter, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let ckp = generate_keypair().unwrap();
        let c = IpcClient::connect(
            uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
        ).await.unwrap();

        // Warmup
        for _ in 0..100 {
            c.send_frame(b"w", Duration::from_secs(5)).await;
        }
        c
    });

    let mut group = c.benchmark_group("control_frame_throughput");
    group.measurement_time(Duration::from_secs(15));

    for &size in &[16usize, 64, 256, 1024, 4096, 65_519] {
        let payload = vec![0xAA; size];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("send_frame", size),
            &payload,
            |b, p| {
                b.iter(|| {
                    rt.block_on(async {
                        let outcome = client.send_frame(p, Duration::from_secs(5)).await;
                        assert!(outcome.is_delivered());
                    });
                });
            },
        );
    }
    group.finish();

    rt.block_on(async { client.shutdown().await });
    let _ = std::fs::remove_file(&path);
}

// ---- Benchmark: bulk + control interleaved ----

fn bench_bulk_with_control_interleaved(c: &mut Criterion) {
    struct SinkRouter;
    impl FrameRouter for SinkRouter {
        fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("interleave");
    let _ = std::fs::remove_file(&path);

    let client = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), SinkRouter, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let ckp = generate_keypair().unwrap();
        let c = IpcClient::connect(
            uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
        ).await.unwrap();

        // Warmup
        c.send_frame(b"w", Duration::from_secs(5)).await;
        let warmup = vec![0u8; 1_000_000];
        c.send_bulk(&warmup, Duration::from_secs(10)).await;
        c
    });

    let bulk_payload = vec![0xBB; 10_000_000]; // 10MB bulk
    let ctrl_payload = vec![0xCC; 256]; // 256B control

    let mut group = c.benchmark_group("bulk_with_control");
    group.throughput(Throughput::Bytes(10_000_000 + 256 * 5)); // bulk + 5 control frames
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("10MB_bulk_plus_5_control", |b| {
        b.iter(|| {
            rt.block_on(async {
                // Send 5 control frames then 1 bulk — simulates chat during file transfer.
                for _ in 0..5 {
                    let o = client.send_frame(&ctrl_payload, Duration::from_secs(5)).await;
                    assert!(o.is_delivered());
                }
                let o = client.send_bulk(&bulk_payload, Duration::from_secs(30)).await;
                assert!(o.is_delivered());
            });
        });
    });
    group.finish();

    rt.block_on(async { client.shutdown().await });
    let _ = std::fs::remove_file(&path);
}

criterion_group!(
    benches,
    bench_connection_rate,
    bench_event_fanout,
    bench_control_frame_throughput,
    bench_bulk_with_control_interleaved,
);
criterion_main!(benches);
