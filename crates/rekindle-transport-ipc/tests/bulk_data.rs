//! Bulk data plane tests: every payload size, boundary, and failure mode.

mod common;

use std::path::PathBuf;
use std::time::Duration;
use bytes::Bytes;
use tokio::sync::mpsc;
use uuid::Uuid;

use rekindle_transport_ipc::bulk::frame::MAX_CHUNK_PLAIN;
use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::noise::keys;
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::*;
use rekindle_transport_ipc::client::IpcClient;

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-bulk-test-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct BulkRouter {
    bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
}
impl FrameRouter for BulkRouter {
    fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
    fn on_bulk_chunk(&self, _: &ServerState, conn_id: u64, stream_id: u8, _chunk_seq: u32, data: &[u8]) {
        self.bulk_accumulator.lock().entry((conn_id, stream_id)).or_default().extend_from_slice(data);
    }
    fn on_bulk_complete(&self, _: &ServerState, conn_id: u64, stream_id: u8, _total_bytes: u64, _total_chunks: u64) {
        let payload = self.bulk_accumulator.lock().remove(&(conn_id, stream_id)).unwrap_or_default();
        let _ = self.bulk_tx.try_send((conn_id, stream_id, payload));
    }
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, old: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, old, new));
    }
}

async fn setup(label: &str) -> (PathBuf, IpcClient, mpsc::Receiver<(u64, u8, Vec<u8>)>, tokio::task::JoinHandle<()>) {
    let path = sock_path(label);
    let _ = std::fs::remove_file(&path);
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (bulk_tx, bulk_rx) = mpsc::channel(64);
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let router = BulkRouter { bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, state_rx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never reached Ready"),
        }
    }
    (path, client, bulk_rx, handle)
}

fn make_payload(size: usize, seed: u8) -> Vec<u8> {
    (0..size).map(|i| ((i % 251) as u8).wrapping_add(seed)).collect()
}

async fn verify_bulk(client: &IpcClient, rx: &mut mpsc::Receiver<(u64, u8, Vec<u8>)>, payload: &[u8], timeout_secs: u64) {
    let outcome = client.send_bulk(payload, Duration::from_secs(timeout_secs)).await;
    assert!(outcome.is_delivered(), "bulk: {outcome:?}");
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert_eq!(received.len(), payload.len(), "size mismatch: {} vs {}", received.len(), payload.len());
    assert_eq!(received, payload, "content mismatch");
}

/// 5.4 Send 0-byte bulk.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_empty() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("bulk-empty").await;
    let _guard = common::SocketGuard::new(path.clone());
    verify_bulk(&client, &mut rx, b"", 10).await;
    client.shutdown().await;
    handle.abort();
}

/// 5.5 Send 1-byte bulk.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_one_byte() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("bulk-1b").await;
    let _guard = common::SocketGuard::new(path.clone());
    verify_bulk(&client, &mut rx, &[0xAA], 10).await;
    client.shutdown().await;
    handle.abort();
}

/// 5.6 Exactly MAX_CHUNK_PLAIN (one chunk, no remainder).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_exact_one_chunk() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("bulk-1chunk").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = make_payload(MAX_CHUNK_PLAIN, 0);
    verify_bulk(&client, &mut rx, &payload, 15).await;
    client.shutdown().await;
    handle.abort();
}

/// 5.7 MAX_CHUNK_PLAIN + 1 (forces exactly 2 chunks).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_two_chunks() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("bulk-2chunk").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = make_payload(MAX_CHUNK_PLAIN + 1, 1);
    verify_bulk(&client, &mut rx, &payload, 15).await;
    client.shutdown().await;
    handle.abort();
}

/// 5.8 Exactly 2 * MAX_CHUNK_PLAIN (2 full chunks, no remainder).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_two_full_chunks() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("bulk-2full").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = make_payload(2 * MAX_CHUNK_PLAIN, 2);
    verify_bulk(&client, &mut rx, &payload, 15).await;
    client.shutdown().await;
    handle.abort();
}

/// 5.9 10 MB bulk.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_10mb() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("bulk-10mb").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = make_payload(10_000_000, 3);
    verify_bulk(&client, &mut rx, &payload, 30).await;
    client.shutdown().await;
    handle.abort();
}

/// 5.11 Three sequential bulk transfers on same connection — no state leakage.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_sequential_transfers() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("bulk-3seq").await;
    let _guard = common::SocketGuard::new(path.clone());

    for i in 0u8..3 {
        let payload = make_payload(100_000, i * 30);
        let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
        assert!(outcome.is_delivered(), "transfer {i}: {outcome:?}");
        let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert_eq!(received.len(), payload.len(), "transfer {i} size");
        assert_eq!(received, payload, "transfer {i} content");
    }

    client.shutdown().await;
    handle.abort();
}

/// 5.18 Sequential control-then-bulk-then-control on same connection.
/// Proves lane multiplexing works when switching between control and bulk
/// in sequence. NOT simultaneous — each phase completes before the next starts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sequential_control_bulk_control() {
    common::init_tracing();
    let path = sock_path("bulk-ctrl-mix");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (bulk_tx, mut bulk_rx) = mpsc::channel(64);
    let (control_tx, mut control_rx) = mpsc::channel(256);
    let (state_tx, mut state_rx) = mpsc::channel(64);

    struct MixRouter {
        bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
        control_tx: mpsc::Sender<(u64, Bytes)>,
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
        bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
    }
    impl FrameRouter for MixRouter {
        fn route_frame(&self, _: &ServerState, id: u64, payload: Bytes) {
            let _ = self.control_tx.try_send((id, payload));
        }
        fn on_bulk_chunk(&self, _: &ServerState, conn_id: u64, stream_id: u8, _chunk_seq: u32, data: &[u8]) {
            self.bulk_accumulator.lock().entry((conn_id, stream_id)).or_default().extend_from_slice(data);
        }
        fn on_bulk_complete(&self, _: &ServerState, conn_id: u64, stream_id: u8, _total_bytes: u64, _total_chunks: u64) {
            let payload = self.bulk_accumulator.lock().remove(&(conn_id, stream_id)).unwrap_or_default();
            let _ = self.bulk_tx.try_send((conn_id, stream_id, payload));
        }
        fn on_connection_state_changed(&self, _: &ServerState, id: u64, old: ConnectionPhase, new: ConnectionPhase) {
            let _ = self.state_tx.try_send((id, old, new));
        }
    }

    let router = MixRouter { bulk_tx, control_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, state_rx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready"),
        }
    }

    // Send 5 control frames first.
    for i in 0..5u32 {
        let outcome = client.send_frame(format!("ctrl-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered());
    }

    // Send a bulk transfer.
    let bulk_payload = make_payload(200_000, 42);
    let bulk_outcome = client.send_bulk(&bulk_payload, Duration::from_secs(15)).await;
    assert!(bulk_outcome.is_delivered());

    // Send 5 more control frames after bulk.
    for i in 5..10u32 {
        let outcome = client.send_frame(format!("ctrl-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered());
    }

    // Verify all 10 control frames.
    for i in 0..10u32 {
        let (_, payload) = tokio::time::timeout(Duration::from_secs(5), control_rx.recv()).await.unwrap().unwrap();
        assert_eq!(std::str::from_utf8(&payload).unwrap(), format!("ctrl-{i}"));
    }

    // Verify bulk.
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), bulk_rx.recv()).await.unwrap().unwrap();
    assert_eq!(received, bulk_payload);

    client.shutdown().await;
    handle.abort();
}
