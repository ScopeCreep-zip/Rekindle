//! Socket-level bulk transfer throughput benchmarks.
//!
//! Measures the ACTUAL production path: client.send_bulk() → real Unix socket
//! → server decrypt → reassembly → on_bulk_chunk (streaming, O(chunk_size)).
//!
//! Every benchmark uses a real Noise IK handshake, real AES-256-GCM encryption,
//! real kernel socket buffers, and real rayon thread pools. No mocks.
//!
//! Run: cargo bench -p rekindle-transport-ipc --bench socket_bulk

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion, Throughput, BenchmarkId};

use rekindle_transport_ipc::client::IpcClient;
use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::noise::keys::generate_keypair;
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::ConnectionPhase;

// ---- Router: streaming sink (O(chunk_size) memory) ----

/// Production-realistic router: counts bytes delivered via on_bulk_chunk.
/// Does NOT accumulate — proves O(chunk_size) memory under sustained bulk.
struct StreamingSinkRouter {
    bytes_delivered: Arc<std::sync::atomic::AtomicU64>,
    transfers_completed: Arc<std::sync::atomic::AtomicU64>,
}

impl FrameRouter for StreamingSinkRouter {
    fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}

    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, data: &[u8]) {
        self.bytes_delivered.fetch_add(data.len() as u64, std::sync::atomic::Ordering::Relaxed);
    }

    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {
        self.transfers_completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
}

// ---- Helpers ----

fn sock_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-bench-{}-{}.sock", label, std::process::id()
    ))
}

fn make_payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}

// ---- Benchmark: client → server bulk throughput at multiple sizes ----

fn bench_bulk_socket_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("bulk-tp");
    let _ = std::fs::remove_file(&path);

    let bytes_delivered = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let transfers_completed = Arc::new(std::sync::atomic::AtomicU64::new(0));

    let client = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let router = StreamingSinkRouter {
            bytes_delivered: Arc::clone(&bytes_delivered),
            transfers_completed: Arc::clone(&transfers_completed),
        };
        // Disable heartbeat for benchmarks — sustained bulk transfers saturate
        // the control loop and can trigger false dead-peer detection.
        let mut config = IpcConfig::default();
        config.heartbeat_interval_ms = 600_000; // 10 minutes — effectively disabled
        config.idle_timeout_ms = 0;             // no idle timeout

        let server = IpcServer::bind(&path, kp.into_inner(), router, config.clone()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let ckp = generate_keypair().unwrap();
        let c = IpcClient::connect(
            uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, None,
        ).await.unwrap();

        // Warmup: one 1MB transfer to prime caches and rayon pool.
        let warmup = make_payload(1_000_000);
        c.send_bulk(&warmup, Duration::from_secs(10)).await;
        c
    });

    let mut group = c.benchmark_group("bulk_socket_throughput");
    group.measurement_time(Duration::from_secs(15));

    for &size in &[65_519usize, 1_000_000, 10_000_000, 50_000_000, 100_000_000] {
        let payload = make_payload(size);
        let label = match size {
            65_519 => "64KB".to_string(),
            1_000_000 => "1MB".to_string(),
            10_000_000 => "10MB".to_string(),
            50_000_000 => "50MB".to_string(),
            100_000_000 => "100MB".to_string(),
            _ => format!("{}B", size),
        };

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("send_bulk", &label),
            &payload,
            |b, p| {
                b.iter(|| {
                    rt.block_on(async {
                        let outcome = client.send_bulk(p, Duration::from_secs(30)).await;
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

// ---- Benchmark: server → client bulk throughput ----

fn bench_bulk_server_to_client(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("bulk-s2c");
    let _ = std::fs::remove_file(&path);

    /// Router that sends bulk back to the client when it receives a control trigger.
    struct EchoBackRouter;
    impl FrameRouter for EchoBackRouter {
        fn route_frame(&self, state: &ServerState, conn_id: u64, payload: Bytes) {
            // Payload is the size to echo back as bulk.
            if payload.len() == 4 {
                let size = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
                if let Some(conn) = state.connections.get(&conn_id) {
                    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
                    let _ = conn.send_bulk(0, &data);
                }
            }
        }
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let mut client = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), EchoBackRouter, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let ckp = generate_keypair().unwrap();
        let mut c = IpcClient::connect(
            uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
        ).await.unwrap();

        // Warmup
        c.send_frame(&1_000_000u32.to_le_bytes(), Duration::from_secs(5)).await;
        let _ = c.recv_bulk().await;
        c
    });

    let mut group = c.benchmark_group("bulk_server_to_client");
    group.measurement_time(Duration::from_secs(15));

    for &size in &[1_000_000usize, 10_000_000, 50_000_000] {
        let label = match size {
            1_000_000 => "1MB",
            10_000_000 => "10MB",
            50_000_000 => "50MB",
            _ => "unknown",
        };

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_function(BenchmarkId::new("recv_bulk", label), |b| {
            b.iter(|| {
                rt.block_on(async {
                    // Trigger server to send bulk back
                    client.send_frame(&(size as u32).to_le_bytes(), Duration::from_secs(5)).await;
                    // Receive via buffered API (benchmark measures full round-trip)
                    let result = client.recv_bulk().await;
                    assert!(result.is_some());
                });
            });
        });
    }
    group.finish();

    rt.block_on(async { client.shutdown().await });
    let _ = std::fs::remove_file(&path);
}

// ---- Benchmark: streaming recv_bulk_chunk latency ----

fn bench_recv_bulk_chunk_streaming(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("bulk-stream");
    let _ = std::fs::remove_file(&path);

    struct TriggerRouter;
    impl FrameRouter for TriggerRouter {
        fn route_frame(&self, state: &ServerState, conn_id: u64, payload: Bytes) {
            if payload.len() == 4 {
                let size = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
                if let Some(conn) = state.connections.get(&conn_id) {
                    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
                    let _ = conn.send_bulk(0, &data);
                }
            }
        }
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let mut client = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), TriggerRouter, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let ckp = generate_keypair().unwrap();
        IpcClient::connect(
            uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
        ).await.unwrap()
    });

    let size = 10_000_000usize; // 10MB = ~153 chunks

    let mut group = c.benchmark_group("bulk_streaming_recv");
    group.throughput(Throughput::Bytes(size as u64));
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("recv_bulk_chunk_10MB", |b| {
        b.iter(|| {
            rt.block_on(async {
                client.send_frame(&(size as u32).to_le_bytes(), Duration::from_secs(5)).await;
                loop {
                    let chunk = client.recv_bulk_chunk().await.unwrap();
                    if chunk.is_last { break; }
                }
            });
        });
    });
    group.finish();

    rt.block_on(async { client.shutdown().await });
    let _ = std::fs::remove_file(&path);
}

criterion_group!(
    benches,
    bench_bulk_socket_throughput,
    bench_bulk_server_to_client,
    bench_recv_bulk_chunk_streaming,
);
criterion_main!(benches);
