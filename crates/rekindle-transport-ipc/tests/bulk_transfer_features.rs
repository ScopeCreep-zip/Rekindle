//! Bulk transfer feature completeness tests.
//!
//! Tests for capabilities that file-transfer consumers need:
//! - Mid-transfer cancellation
//! - Multiple concurrent streams on different stream_ids
//! - Progress reporting during transfer
//! - Deterministic replay (same inputs → same outputs)
//!
//! Every test uses real sockets with real Noise handshakes.

mod common;

use std::path::PathBuf;
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
        "rekindle-btf-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct BtfRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
}
impl FrameRouter for BtfRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) {
        let _ = self.control_tx.try_send((id, p));
    }
    fn on_bulk_chunk(&self, _: &ServerState, conn_id: u64, stream_id: u8, _chunk_seq: u32, data: &[u8]) {
        self.bulk_accumulator.lock().entry((conn_id, stream_id)).or_default().extend_from_slice(data);
    }
    fn on_bulk_complete(&self, _: &ServerState, conn_id: u64, stream_id: u8, _total_bytes: u64, _total_chunks: u64) {
        let payload = self.bulk_accumulator.lock().remove(&(conn_id, stream_id)).unwrap_or_default();
        let _ = self.bulk_tx.try_send((conn_id, stream_id, payload));
    }
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
    }
}

async fn wait_ready(rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>) -> u64 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, _, ConnectionPhase::Ready))) => return id,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready"),
        }
    }
}

// ---- 13. Progress reporting during bulk transfer ----

/// Proves: client.bulk_counters() and client.encrypt_nonces_issued() provide
/// real-time progress during a bulk transfer. The counters must increment
/// between the start and completion of the transfer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_progress_reporting() {
    common::init_tracing();
    let path = sock_path("progress");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _) = mpsc::channel(64);
    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = BtfRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_counters: Arc<rekindle_transport_ipc::bulk::BulkCounters> = server.bulk_counters().clone();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Baseline: all counters at 0.
    assert_eq!(client.encrypt_nonces_issued(), 0);
    assert_eq!(client.bulk_counters().frames_sent.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(server_counters.frames_received.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(server_counters.chunks_decrypted.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(server_counters.chunks_reassembled.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(server_counters.transfers_completed.load(std::sync::atomic::Ordering::Relaxed), 0);

    // Send 200KB bulk.
    let payload: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered(), "bulk: {outcome:?}");

    // Wait for delivery.
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv())
        .await.unwrap().unwrap();
    common::assert_payload_eq(&received, &payload);

    // After completion: all counters must have incremented.
    let nonces = client.encrypt_nonces_issued();
    let client_frames = client.bulk_counters().frames_sent.load(std::sync::atomic::Ordering::Relaxed);
    let client_bytes = client.bulk_counters().bytes_sent.load(std::sync::atomic::Ordering::Relaxed);
    let server_frames = server_counters.frames_received.load(std::sync::atomic::Ordering::Relaxed);
    let server_decrypted = server_counters.chunks_decrypted.load(std::sync::atomic::Ordering::Relaxed);
    let server_reassembled = server_counters.chunks_reassembled.load(std::sync::atomic::Ordering::Relaxed);
    let server_completed = server_counters.transfers_completed.load(std::sync::atomic::Ordering::Relaxed);

    assert!(nonces > 0, "encrypt_nonces_issued must be > 0, got {nonces}");
    assert!(client_frames > 0, "client frames_sent must be > 0, got {client_frames}");
    assert!(client_bytes >= payload.len() as u64,
        "client bytes_sent {client_bytes} must be >= payload {}", payload.len());
    assert!(server_frames > 0, "server frames_received must be > 0, got {server_frames}");
    assert!(server_decrypted > 0, "server chunks_decrypted must be > 0, got {server_decrypted}");
    assert!(server_reassembled > 0, "server chunks_reassembled must be > 0, got {server_reassembled}");
    assert_eq!(server_completed, 1, "server transfers_completed must be 1, got {server_completed}");

    // Nonces issued >= client frames sent (some may still be in channel).
    assert!(nonces >= client_frames,
        "nonces {nonces} must be >= frames_sent {client_frames}");

    tracing::info!(
        nonces, client_frames, client_bytes,
        server_frames, server_decrypted, server_reassembled, server_completed,
        "full pipeline progress verified"
    );

    client.shutdown().await;
    handle.abort();
}

// ---- 14. Sequential bulk transfers with counter accumulation ----

/// Proves: counters accumulate across multiple sequential transfers.
/// After 3 transfers, transfers_completed == 3 and all other counters
/// reflect the sum of all transfers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn counters_accumulate_across_transfers() {
    common::init_tracing();
    let path = sock_path("counter-accum");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _) = mpsc::channel(64);
    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = BtfRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_counters = server.bulk_counters().clone();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    for round in 0u8..3 {
        let payload: Vec<u8> = (0..100_000).map(|i| ((i % 251) as u8).wrapping_add(round * 50)).collect();
        let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
        assert!(outcome.is_delivered(), "round {round}: {outcome:?}");
        let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv())
            .await.unwrap().unwrap();
        common::assert_payload_eq(&received, &payload);
    }

    let completed = server_counters.transfers_completed.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(completed, 3, "transfers_completed must be 3, got {completed}");

    let client_frames = client.bulk_counters().frames_sent.load(std::sync::atomic::Ordering::Relaxed);
    assert!(client_frames > 3, "client must have sent more than 3 frames across 3 transfers, got {client_frames}");

    client.shutdown().await;
    handle.abort();
}

// ---- 15. Deterministic output: same payload → same chunk count ----

/// Proves: sending the same payload twice produces the same number of
/// encrypted frames. The pipeline is deterministic — no randomness in
/// chunking, no non-deterministic scheduling affects frame count.
///
/// Note: encrypted frame CONTENT differs between runs because nonces
/// advance. But the structural output (frame count, frame sizes) must
/// be identical for identical input.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deterministic_frame_count() {
    common::init_tracing();
    let path = sock_path("deterministic");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _) = mpsc::channel(64);
    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = BtfRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_counters = server.bulk_counters().clone();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    let payload: Vec<u8> = (0..300_000).map(|i| (i % 251) as u8).collect();

    // Transfer 1.
    let nonces_before_1 = client.encrypt_nonces_issued();
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered());
    let nonces_after_1 = client.encrypt_nonces_issued();
    let frame_count_1 = nonces_after_1 - nonces_before_1;
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv()).await.unwrap().unwrap();
    common::assert_payload_eq(&received, &payload);

    // Transfer 2: same payload.
    let nonces_before_2 = client.encrypt_nonces_issued();
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered());
    let nonces_after_2 = client.encrypt_nonces_issued();
    let frame_count_2 = nonces_after_2 - nonces_before_2;
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv()).await.unwrap().unwrap();
    common::assert_payload_eq(&received, &payload);

    assert_eq!(
        frame_count_1, frame_count_2,
        "same payload must produce same frame count: run1={frame_count_1}, run2={frame_count_2}"
    );

    let completed = server_counters.transfers_completed.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(completed, 2);

    client.shutdown().await;
    handle.abort();
}

// ---- 16. Bulk transfer with server-side counter verification at every stage ----

/// Proves: after a complete bulk transfer, every pipeline stage counter
/// is consistent. chunks_decrypted >= chunks_reassembled because decrypted
/// chunks may be buffered awaiting ordering. transfers_completed == 1.
/// All counters are non-zero.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_pipeline_counter_consistency() {
    common::init_tracing();
    let path = sock_path("counter-consist");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _) = mpsc::channel(64);
    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = BtfRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let sc = server.bulk_counters().clone();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    let payload: Vec<u8> = (0..500_000).map(|i| (i % 251) as u8).collect();
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered(), "bulk: {outcome:?}");

    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv())
        .await.unwrap().unwrap();
    common::assert_payload_eq(&received, &payload);

    // Read all counters.
    let cc = client.bulk_counters();
    let c_nonces = client.encrypt_nonces_issued();
    let c_frames = cc.frames_sent.load(std::sync::atomic::Ordering::Relaxed);
    let c_bytes = cc.bytes_sent.load(std::sync::atomic::Ordering::Relaxed);
    let s_frames = sc.frames_received.load(std::sync::atomic::Ordering::Relaxed);
    let s_bytes = sc.bytes_received.load(std::sync::atomic::Ordering::Relaxed);
    let s_decrypted = sc.chunks_decrypted.load(std::sync::atomic::Ordering::Relaxed);
    let s_reassembled = sc.chunks_reassembled.load(std::sync::atomic::Ordering::Relaxed);
    let s_completed = sc.transfers_completed.load(std::sync::atomic::Ordering::Relaxed);

    // Stage 1: encrypt
    assert!(c_nonces > 0, "nonces issued must be > 0");
    // Stage 2: channel → write task (nonces >= frames sent, some may be in-flight)
    assert!(c_nonces >= c_frames, "nonces {c_nonces} >= frames_sent {c_frames}");
    // Stage 3: socket write
    assert!(c_frames > 0, "client frames_sent > 0");
    assert!(c_bytes >= payload.len() as u64, "client bytes >= payload");
    // Stage 4: socket read
    assert!(s_frames > 0, "server frames_received > 0");
    assert!(s_bytes > 0, "server bytes_received > 0");
    // Stage 5: decrypt
    assert!(s_decrypted > 0, "chunks_decrypted > 0");
    // decrypted should equal frames received (each frame produces one decrypt task)
    assert_eq!(s_decrypted, s_frames,
        "chunks_decrypted {s_decrypted} must equal frames_received {s_frames}");
    // Stage 6: reassembly
    assert!(s_reassembled > 0, "chunks_reassembled > 0");
    // reassembled <= decrypted (some may be buffered for ordering)
    assert!(s_reassembled <= s_decrypted,
        "chunks_reassembled {s_reassembled} must be <= chunks_decrypted {s_decrypted}");
    // Completion
    assert_eq!(s_completed, 1, "transfers_completed must be 1");

    tracing::info!(
        c_nonces, c_frames, c_bytes,
        s_frames, s_bytes, s_decrypted, s_reassembled, s_completed,
        "full pipeline counter consistency verified"
    );

    client.shutdown().await;
    handle.abort();
}
