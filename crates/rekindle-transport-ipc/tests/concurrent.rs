//! Concurrent I/O tests: multiple clients, bidirectional traffic,
//! simultaneous bulk and control, fan-out from server.
//!
//! Proves the transport handles parallel load without deadlock,
//! corruption, or cross-contamination.

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc;
use uuid::Uuid;

use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::noise::keys;
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::*;
use rekindle_transport_ipc::client::IpcClient;

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-concurrent-test-{}-{}-{}.sock",
        label, std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_nanos()
    ))
}

struct ConcurrentRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
    frame_count: Arc<AtomicU64>,
    bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
}

impl FrameRouter for ConcurrentRouter {
    fn route_frame(&self, _: &ServerState, conn_id: u64, payload: Bytes) {
        self.frame_count.fetch_add(1, Ordering::Relaxed);
        let _ = self.control_tx.try_send((conn_id, payload));
    }
    fn on_bulk_chunk(&self, _: &ServerState, conn_id: u64, stream_id: u8, _chunk_seq: u32, data: &[u8]) {
        self.bulk_accumulator.lock().entry((conn_id, stream_id)).or_default().extend_from_slice(data);
    }
    fn on_bulk_complete(&self, _: &ServerState, conn_id: u64, stream_id: u8, _total_bytes: u64, _total_chunks: u64) {
        let payload = self.bulk_accumulator.lock().remove(&(conn_id, stream_id)).unwrap_or_default();
        let _ = self.bulk_tx.try_send((conn_id, stream_id, payload));
    }
    fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
}

/// Proves: 4 clients connect simultaneously, each sends 25 control frames,
/// all 100 frames arrive, no cross-contamination.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn four_clients_control_plane() {
    common::init_tracing();
    let path = sock_path("4-ctrl");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let frame_count = Arc::new(AtomicU64::new(0));
    let (control_tx, mut control_rx) = mpsc::channel::<(u64, Bytes)>(1024);
    let (bulk_tx, _) = mpsc::channel(64);
    let router = ConcurrentRouter {
        control_tx,
        bulk_tx,
        frame_count: Arc::clone(&frame_count),
        bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()),
    };

    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let mut handles = Vec::new();
    for idx in 0u8..4 {
        let path = path.clone();
        let server_pub = server_pub;
        handles.push(tokio::spawn(async move {
            let kp = keys::generate_keypair().unwrap();
            let client = IpcClient::connect(
                Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None,
            ).await.unwrap();

            for i in 0u32..25 {
                let msg = format!("c{idx}-m{i}");
                let outcome = client.send_frame(msg.as_bytes(), Duration::from_secs(5)).await;
                assert!(
                    outcome.is_delivered(),
                    "client {idx} frame {i}: expected Delivered, got {outcome:?}"
                );
            }
            client.shutdown().await;
        }));
    }

    // Wait for all clients to complete. Each send_frame awaited Delivered,
    // so all 100 frames are acked and therefore delivered to the router.
    for h in handles {
        h.await.expect("client task panicked");
    }

    // Verify the count.
    assert_eq!(
        frame_count.load(Ordering::Relaxed), 100,
        "expected 100 frames, router received {}",
        frame_count.load(Ordering::Relaxed)
    );

    // Drain and verify each frame has correct format.
    let mut received = Vec::new();
    while let Ok(Some((_, payload))) = tokio::time::timeout(Duration::from_millis(200), control_rx.recv()).await {
        received.push(payload);
    }
    assert_eq!(received.len(), 100, "expected 100 frames in channel, got {}", received.len());

    // Every payload should match pattern "cN-mM".
    for payload in &received {
        let s = std::str::from_utf8(payload).expect("payload not utf8");
        assert!(s.starts_with('c'), "unexpected payload: {s}");
        assert!(s.contains("-m"), "unexpected payload format: {s}");
    }

    server_handle.abort();
}

/// Proves: 4 clients each do a bulk transfer simultaneously.
/// All 4 payloads arrive byte-identical, no cross-contamination.
/// Shared rayon pool handles concurrent encrypt without deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn four_clients_concurrent_bulk() {
    common::init_tracing();
    let path = sock_path("4-bulk");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let (control_tx, _) = mpsc::channel(64);
    let (bulk_tx, mut bulk_rx) = mpsc::channel::<(u64, u8, Vec<u8>)>(64);
    let router = ConcurrentRouter {
        control_tx,
        bulk_tx,
        frame_count: Arc::new(AtomicU64::new(0)),
        bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()),
    };

    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let mut handles = Vec::new();
    for idx in 0u8..4 {
        let path = path.clone();
        let server_pub = server_pub;
        handles.push(tokio::spawn(async move {
            let kp = keys::generate_keypair().unwrap();
            let client = IpcClient::connect(
                Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None,
            ).await.unwrap();

            // Each client's payload has a unique seed.
            let payload: Vec<u8> = (0..100_000)
                .map(|i| ((i % 251) as u8).wrapping_add(idx * 50))
                .collect();

            let outcome = client.send_bulk(&payload, Duration::from_secs(30)).await;
            assert!(
                outcome.is_delivered(),
                "client {idx} bulk: expected Delivered, got {outcome:?}"
            );

            client.shutdown().await;
            payload // return for verification
        }));
    }

    // Collect expected payloads.
    let mut expected: Vec<Vec<u8>> = Vec::new();
    for h in handles {
        expected.push(h.await.expect("client task panicked"));
    }

    // Collect delivered payloads.
    let mut delivered: Vec<Vec<u8>> = Vec::new();
    while let Ok(Some((_, _, payload))) = tokio::time::timeout(Duration::from_millis(500), bulk_rx.recv()).await {
        delivered.push(payload);
    }

    assert_eq!(delivered.len(), 4, "expected 4 bulk payloads, got {}", delivered.len());

    // Every delivered payload must match exactly one expected payload.
    for recv in &delivered {
        assert_eq!(recv.len(), 100_000, "payload size wrong: {}", recv.len());
        let matched = expected.iter().any(|exp| exp == recv);
        assert!(matched, "delivered payload matches no expected (first byte: {})", recv[0]);
    }

    // No two delivered payloads are identical (unique seeds).
    for i in 0..delivered.len() {
        for j in (i + 1)..delivered.len() {
            assert_ne!(delivered[i], delivered[j], "payloads {i} and {j} are identical — cross-contamination");
        }
    }

    server_handle.abort();
}

/// Proves: mixed control + bulk from multiple clients simultaneously.
/// Each client sends 10 control frames AND one 50KB bulk payload.
/// All 40 control frames and 4 bulk payloads arrive correctly.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn four_clients_mixed_control_and_bulk() {
    common::init_tracing();
    let path = sock_path("4-mixed");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let frame_count = Arc::new(AtomicU64::new(0));
    let (control_tx, mut control_rx) = mpsc::channel::<(u64, Bytes)>(1024);
    let (bulk_tx, mut bulk_rx) = mpsc::channel::<(u64, u8, Vec<u8>)>(64);
    let router = ConcurrentRouter {
        control_tx,
        bulk_tx,
        frame_count: Arc::clone(&frame_count),
        bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()),
    };

    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let mut handles = Vec::new();
    for idx in 0u8..4 {
        let path = path.clone();
        let server_pub = server_pub;
        handles.push(tokio::spawn(async move {
            let kp = keys::generate_keypair().unwrap();
            let client = IpcClient::connect(
                Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None,
            ).await.unwrap();

            // Send 10 control frames.
            for i in 0u32..10 {
                let msg = format!("c{idx}-m{i}");
                let outcome = client.send_frame(msg.as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "client {idx} frame {i}: {outcome:?}");
            }

            // Send one bulk payload.
            let payload: Vec<u8> = (0..50_000)
                .map(|i| ((i % 251) as u8).wrapping_add(idx * 60))
                .collect();
            let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
            assert!(outcome.is_delivered(), "client {idx} bulk: {outcome:?}");

            client.shutdown().await;
        }));
    }

    for h in handles {
        h.await.expect("client task panicked");
    }

    // All sends returned Delivered, so all frames are at the router.
    assert_eq!(frame_count.load(Ordering::Relaxed), 40, "expected 40 control frames");

    let mut control_count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(200), control_rx.recv()).await {
        control_count += 1;
    }
    assert_eq!(control_count, 40);

    let mut bulk_count = 0;
    while let Ok(Some((_, _, payload))) = tokio::time::timeout(Duration::from_millis(500), bulk_rx.recv()).await {
        assert_eq!(payload.len(), 50_000);
        bulk_count += 1;
    }
    assert_eq!(bulk_count, 4);

    server_handle.abort();
}

/// Proves: echo roundtrip on a single connection — client sends 20 frames
/// sequentially (each awaiting Delivered), then receives all 20 echoes.
/// This is SEQUENTIAL send-then-receive, not simultaneous bidirectional.
/// For true simultaneous bidirectional, see control_recv.rs::true_bidirectional_simultaneous.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn echo_roundtrip_sequential() {
    common::init_tracing();
    let path = sock_path("bidir");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    // This router echoes received frames back to the sender's response_tx.
    struct EchoRouter {
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    }

    impl FrameRouter for EchoRouter {
        fn route_frame(&self, state: &ServerState, conn_id: u64, payload: Bytes) {
            // Echo the payload back to the sender via their response_tx.
            if let Some(conn) = state.connections.get(&conn_id) {
                let _ = conn.response_tx.try_send(payload);
            }
        }
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, conn_id: u64, old: ConnectionPhase, new: ConnectionPhase) {
            let _ = self.state_tx.try_send((conn_id, old, new));
        }
    }

    let (state_tx, mut state_rx) = mpsc::channel(64);
    let router = EchoRouter { state_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let kp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    // Wait for Ready.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, state_rx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never reached Ready"),
        }
    }

    // Send 20 frames and simultaneously receive echoes.
    let send_task = {
        // We need a reference to the client for sending, but also for receiving.
        // Use separate channels: send via app_tx, receive via inbound_rx.
        // But the client API doesn't split this way. Instead, send all frames
        // first (each awaiting Delivered), then collect echoes.
        let mut outcomes = Vec::new();
        for i in 0u32..20 {
            let msg = format!("echo-{i}");
            let outcome = client.send_frame(msg.as_bytes(), Duration::from_secs(5)).await;
            assert!(outcome.is_delivered(), "frame {i}: {outcome:?}");
            outcomes.push(i);
        }
        outcomes
    };

    assert_eq!(send_task.len(), 20);

    // Now receive the 20 echoes. The server echoed each to response_tx,
    // which the client's IO loop delivers to inbound_rx.
    for i in 0u32..20 {
        let payload = tokio::time::timeout(Duration::from_secs(5), client.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for echo {i}"))
            .unwrap_or_else(|| panic!("channel closed waiting for echo {i}"));
        let expected = format!("echo-{i}");
        assert_eq!(
            std::str::from_utf8(&payload).unwrap_or("<not utf8>"),
            expected,
            "echo {i} content mismatch"
        );
    }

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: many messages on one connection (500 control frames).
/// Verifies the transport sustains throughput without degradation or deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sustained_500_frames_one_connection() {
    common::init_tracing();
    let path = sock_path("500-frames");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let frame_count = Arc::new(AtomicU64::new(0));
    let (control_tx, mut control_rx) = mpsc::channel::<(u64, Bytes)>(1024);
    let (bulk_tx, _) = mpsc::channel(64);
    let router = ConcurrentRouter {
        control_tx,
        bulk_tx,
        frame_count: Arc::clone(&frame_count),
        bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()),
    };

    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    let start = std::time::Instant::now();
    for i in 0u64..500 {
        let msg = format!("sustained-{i}");
        let outcome = client.send_frame(msg.as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "frame {i}: {outcome:?}");
    }
    let elapsed = start.elapsed();

    tracing::info!(
        frames = 500,
        elapsed_ms = elapsed.as_millis() as u64,
        frames_per_sec = (500.0 / elapsed.as_secs_f64()) as u64,
        "sustained send complete"
    );

    assert_eq!(frame_count.load(Ordering::Relaxed), 500);

    // Verify all 500 arrived.
    let mut count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(200), control_rx.recv()).await {
        count += 1;
    }
    assert_eq!(count, 500, "expected 500 frames in channel, got {count}");

    client.shutdown().await;
    server_handle.abort();
}
