//! Socket-level concurrency and scaling benchmarks.
//!
//! Measures how throughput scales with multiple concurrent clients,
//! parallel bulk streams, and mixed workloads. Identifies contention
//! points in the rayon pool, channel infrastructure, and server dispatch.
//!
//! Run: cargo bench -p rekindle-transport-ipc --bench socket_concurrency

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion, Throughput, BenchmarkId};

use rekindle_transport_ipc::client::{IpcClient, SharedPools};
use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::noise::keys::generate_keypair;
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::ConnectionPhase;

fn sock_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-bench-{}-{}.sock", label, std::process::id()
    ))
}

struct SinkRouter;
impl FrameRouter for SinkRouter {
    fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
}

// ---- Benchmark: N clients concurrent bulk ----

fn bench_concurrent_bulk_clients(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("conc-bulk");
    let _ = std::fs::remove_file(&path);

    let mut config = IpcConfig::default();
    config.encrypt_workers = 4;
    config.heartbeat_interval_ms = 600_000;
    config.idle_timeout_ms = 0;
    let shared = SharedPools::new(&config);

    let clients: Vec<Arc<IpcClient>> = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), SinkRouter, config.clone()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let mut cs = Vec::new();
        for _ in 0..8 {
            let ckp = generate_keypair().unwrap();
            let c = IpcClient::connect(
                uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, Some(&shared),
            ).await.unwrap();
            let w = vec![0u8; 100_000];
            c.send_bulk(&w, Duration::from_secs(10)).await;
            cs.push(Arc::new(c));
        }
        cs
    });

    let payload = Arc::new(vec![0xAA; 10_000_000]); // 10MB per client

    let mut group = c.benchmark_group("concurrent_bulk_clients");
    group.measurement_time(Duration::from_secs(20));

    for &n in &[1usize, 2, 4, 8] {
        // Total throughput = n clients × 10MB
        group.throughput(Throughput::Bytes(n as u64 * 10_000_000));
        group.bench_with_input(
            BenchmarkId::new("clients", n),
            &n,
            |b, &num| {
                b.iter(|| {
                    rt.block_on(async {
                        let mut tasks = Vec::new();
                        for client in clients.iter().take(num) {
                            let c = Arc::clone(client);
                            let p = Arc::clone(&payload);
                            tasks.push(tokio::spawn(async move {
                                let o = c.send_bulk(&p, Duration::from_secs(30)).await;
                                assert!(o.is_delivered());
                            }));
                        }
                        for t in tasks { t.await.unwrap(); }
                    });
                });
            },
        );
    }
    group.finish();

    drop(clients);
    let _ = std::fs::remove_file(&path);
}

// ---- Benchmark: N clients concurrent control frames ----

fn bench_concurrent_control_scaling(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("conc-ctrl");
    let _ = std::fs::remove_file(&path);

    let mut config = IpcConfig::default();
    config.encrypt_workers = 4;
    config.heartbeat_interval_ms = 600_000;
    config.idle_timeout_ms = 0;
    let shared = SharedPools::new(&config);

    let clients: Vec<Arc<IpcClient>> = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), SinkRouter, config.clone()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let mut cs = Vec::new();
        for _ in 0..8 {
            let ckp = generate_keypair().unwrap();
            let c = IpcClient::connect(
                uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, Some(&shared),
            ).await.unwrap();
            for _ in 0..10 { c.send_frame(b"w", Duration::from_secs(5)).await; }
            cs.push(Arc::new(c));
        }
        cs
    });

    let payload = vec![0xBB; 256];
    let frames_per_client = 20;

    let mut group = c.benchmark_group("concurrent_control_scaling");
    group.measurement_time(Duration::from_secs(25));

    for &n in &[1usize, 2, 4, 8] {
        group.throughput(Throughput::Elements(n as u64 * frames_per_client));
        group.bench_with_input(
            BenchmarkId::new("clients", n),
            &n,
            |b, &num| {
                b.iter(|| {
                    rt.block_on(async {
                        let mut tasks = Vec::new();
                        for client in clients.iter().take(num) {
                            let c = Arc::clone(client);
                            let p = payload.clone();
                            tasks.push(tokio::spawn(async move {
                                for _ in 0..frames_per_client {
                                    c.send_frame(&p, Duration::from_secs(5)).await;
                                }
                            }));
                        }
                        for t in tasks { t.await.unwrap(); }
                    });
                });
            },
        );
    }
    group.finish();

    drop(clients);
    let _ = std::fs::remove_file(&path);
}

// ---- Benchmark: parallel streams on one connection ----

fn bench_parallel_streams(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("par-streams");
    let _ = std::fs::remove_file(&path);

    let mut config = IpcConfig::default();
    config.encrypt_workers = 4;
    config.heartbeat_interval_ms = 600_000;
    config.idle_timeout_ms = 0;

    let client: Arc<IpcClient> = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), SinkRouter, config.clone()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let ckp = generate_keypair().unwrap();
        let c = IpcClient::connect(
            uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, None,
        ).await.unwrap();

        // Warmup
        let w = vec![0u8; 1_000_000];
        c.send_bulk(&w, Duration::from_secs(10)).await;
        Arc::new(c)
    });

    let payload = Arc::new(vec![0xCC; 5_000_000]); // 5MB per stream

    let mut group = c.benchmark_group("parallel_streams");
    group.measurement_time(Duration::from_secs(100));

    // N parallel streams on the SAME connection (different stream_ids).
    for &n in &[1u8, 2, 4] {
        group.throughput(Throughput::Bytes(n as u64 * 5_000_000));
        group.bench_function(BenchmarkId::new("streams", n), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let mut tasks = Vec::new();
                    for stream_id in 0..n {
                        let c = Arc::clone(&client);
                        let p = Arc::clone(&payload);
                        tasks.push(tokio::spawn(async move {
                            let o = c.send_bulk_on_stream(stream_id, &p, Duration::from_secs(30)).await;
                            assert!(o.is_delivered());
                        }));
                    }
                    for t in tasks { t.await.unwrap(); }
                });
            });
        });
    }
    group.finish();

    drop(client);
    let _ = std::fs::remove_file(&path);
}

// ---- Benchmark: sustained throughput (many sequential transfers) ----

fn bench_sustained_bulk(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let path = sock_path("sustained");
    let _ = std::fs::remove_file(&path);

    let mut config = IpcConfig::default();
    config.encrypt_workers = 4;
    config.heartbeat_interval_ms = 600_000;
    config.idle_timeout_ms = 0;

    let client = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), SinkRouter, config.clone()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let ckp = generate_keypair().unwrap();
        let c = IpcClient::connect(
            uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, None,
        ).await.unwrap();

        // Warmup
        let w = vec![0u8; 1_000_000];
        c.send_bulk(&w, Duration::from_secs(10)).await;
        c
    });

    // 10 sequential 10MB transfers per iteration = 100MB sustained.
    let payload = vec![0xDD; 10_000_000];
    let transfers_per_iter = 10;

    let mut group = c.benchmark_group("sustained_bulk");
    group.throughput(Throughput::Bytes(transfers_per_iter * 10_000_000));
    group.measurement_time(Duration::from_secs(25));
    group.sample_size(20);

    group.bench_function("10x10MB_sequential", |b| {
        b.iter(|| {
            rt.block_on(async {
                for _ in 0..transfers_per_iter {
                    let o = client.send_bulk(&payload, Duration::from_secs(30)).await;
                    assert!(o.is_delivered());
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
    bench_concurrent_bulk_clients,
    bench_concurrent_control_scaling,
    bench_parallel_streams,
    bench_sustained_bulk,
);
criterion_main!(benches);
