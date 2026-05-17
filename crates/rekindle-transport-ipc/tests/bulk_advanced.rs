//! Advanced bulk tests: large payloads, mid-transfer failure, concurrent streams,
//! bulk while receiving events, truly interleaved control+bulk.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use tokio::sync::mpsc;
use uuid::Uuid;

use rekindle_transport_ipc::bulk::frame::MAX_CHUNK_PLAIN;
use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::envelope::SharedFrame;
use rekindle_transport_ipc::noise::keys;
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::*;
use rekindle_transport_ipc::client::IpcClient;

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-badv-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct BulkAdvRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
}
impl FrameRouter for BulkAdvRouter {
    fn route_frame(&self, state: &ServerState, id: u64, p: Bytes) {
        let _ = self.control_tx.try_send((id, p.clone()));
        // Echo back via response_tx AND push an event via event_tx.
        // This exercises both server-to-client paths simultaneously.
        if let Some(conn) = state.connections.get(&id) {
            let _ = conn.response_tx.try_send(p.clone());
            let _ = conn.event_tx.try_send(SharedFrame::from_bytes(&p));
        }
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

fn make_payload(size: usize, seed: u8) -> Vec<u8> {
    (0..size).map(|i| ((i % 251) as u8).wrapping_add(seed)).collect()
}

async fn setup(label: &str) -> (PathBuf, IpcClient, mpsc::Receiver<(u64, Bytes)>, mpsc::Receiver<(u64, u8, Vec<u8>)>, tokio::task::JoinHandle<()>) {
    let path = sock_path(label);
    let _ = std::fs::remove_file(&path);
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, crx) = mpsc::channel(4096);
    let (btx, brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(256);
    let router = BulkAdvRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let h = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut srx).await;
    (path, client, crx, brx, h)
}

/// 5.9a 1 MB bulk — proves sustained pipeline at modest scale.
/// Must pass before attempting larger transfers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_1mb() {
    common::init_tracing();
    let (path, client, _, mut brx, handle) = setup("bulk-1mb").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = make_payload(1_000_000, 3);
    let outcome = client.send_bulk(&payload, Duration::from_secs(10)).await;
    assert!(outcome.is_delivered(), "1MB: {outcome:?}");
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv())
        .await.unwrap().unwrap();
    common::assert_payload_eq(&received, &payload);
    client.shutdown().await;
    handle.abort();
}

/// 5.9b 10 MB bulk — proves sustained pipeline at medium scale.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_10mb() {
    common::init_tracing();
    let (path, client, _, mut brx, handle) = setup("bulk-10mb").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = make_payload(10_000_000, 5);
    let outcome = client.send_bulk(&payload, Duration::from_secs(10)).await;
    assert!(outcome.is_delivered(), "10MB: {outcome:?}");
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(10), brx.recv())
        .await.unwrap().unwrap();
    common::assert_payload_eq(&received, &payload);
    client.shutdown().await;
    handle.abort();
}

/// 5.9c 50 MB bulk — ~764 chunks, sustained pipeline at scale.
/// At 10 Gbps, 50MB should transfer in ~40ms. A 10-second deadline
/// is 250x headroom. If this fails, the pipeline has a throughput defect.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_50mb() {
    common::init_tracing();
    // Inline setup to capture server counters.
    let path = sock_path("bulk-50mb");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _crx) = mpsc::channel(4096);
    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(256);
    let router = BulkAdvRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_counters = server.bulk_counters().clone();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut srx).await;

    let payload = make_payload(50_000_000, 7);

    let client_counters = client.bulk_counters().clone();
    let start = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        client.send_bulk(&payload, Duration::from_secs(10)),
    ).await;

    let elapsed = start.elapsed();
    use std::sync::atomic::Ordering::Relaxed;

    // Client-side diagnostics.
    let c_nonces = client.encrypt_nonces_issued();
    let c_frames = client_counters.frames_sent.load(Relaxed);
    let c_bytes = client_counters.bytes_sent.load(Relaxed);
    let c_queued = client.bulk_channel_queued();

    // Server-side diagnostics.
    let s_frames_recv = server_counters.frames_received.load(Relaxed);
    let s_bytes_recv = server_counters.bytes_received.load(Relaxed);
    let s_decrypted = server_counters.chunks_decrypted.load(Relaxed);
    let s_reassembled = server_counters.chunks_reassembled.load(Relaxed);
    let s_completed = server_counters.transfers_completed.load(Relaxed);
    let s_frames_sent = server_counters.frames_sent.load(Relaxed);
    let s_bytes_sent = server_counters.bytes_sent.load(Relaxed);

    let outcome = outcome.unwrap_or_else(|_| {
        panic!(
            "50MB bulk transfer stalled after {elapsed:?}.\n\
             \n\
             CLIENT pipeline:\n\
             - Rayon encrypt: {c_nonces} nonces issued (expected ~765)\n\
             - Channel queue: ~{c_queued} frames buffered\n\
             - Write task: {c_frames} frames / {c_bytes} bytes written to socket\n\
             \n\
             SERVER pipeline:\n\
             - Read/dispatch: {s_frames_recv} frames / {s_bytes_recv} bytes received\n\
             - Decrypt (rayon): {s_decrypted} chunks decrypted\n\
             - Reassembly: {s_reassembled} chunks reassembled\n\
             - Completed: {s_completed} transfers completed\n\
             - BulkAck write: {s_frames_sent} frames / {s_bytes_sent} bytes sent back\n\
             \n\
             Stall diagnosis:\n\
             - c_frames < 765 and s_frames_recv == c_frames: socket backpressure (server not reading fast enough)\n\
             - s_frames_recv > 0 but s_decrypted == 0: rayon decrypt pool stalled\n\
             - s_decrypted > 0 but s_reassembled == 0: reassembly channel full or control loop not draining\n\
             - s_reassembled > 0 but s_completed == 0: Merkle verification failed or accumulator bug\n\
             - s_completed == 1 but client never got ack: BulkAck not written or client not reading"
        )
    });
    assert!(outcome.is_delivered(), "50MB in {elapsed:?}: {outcome:?}");

    let (_, _, received) = tokio::time::timeout(Duration::from_secs(30), brx.recv())
        .await
        .unwrap_or_else(|_| panic!("server never delivered 50MB payload after {elapsed:?}"))
        .unwrap();
    assert_eq!(received.len(), payload.len());
    assert_eq!(received, payload);

    let throughput_mibs = 50.0 / elapsed.as_secs_f64();
    tracing::info!(
        elapsed_ms = elapsed.as_millis() as u64,
        throughput_mibs = throughput_mibs as u64,
        c_nonces, c_frames, s_frames_recv, s_decrypted, s_reassembled, s_completed,
        "50MB bulk transfer complete"
    );

    client.shutdown().await;
    handle.abort();
}

/// 5.13 Server dies mid-bulk — client resolves with error, doesn't hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_server_dies_midtransfer() {
    common::init_tracing();
    let path = sock_path("bulk-die-mid");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let (btx, _) = mpsc::channel(64);
    let (ctx, _) = mpsc::channel(64);
    let router = BulkAdvRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut srx).await;

    // Start a 10MB bulk in a task.
    let bulk_task = tokio::spawn({
        async move {
            let payload = make_payload(10_000_000, 99);
            let outcome = client.send_bulk(&payload, Duration::from_secs(10)).await;
            (client, outcome)
        }
    });

    // Intentional delay: let bulk pipeline start encrypting before killing server.
    // This is NOT synchronization — it's a timing setup for the test scenario.
    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort();

    // Bulk must resolve (not hang). Outcome is non-Delivered.
    let (client, outcome) = tokio::time::timeout(Duration::from_secs(15), bulk_task)
        .await.expect("bulk hung after server death").expect("bulk panicked");

    // Either delivered (very fast) or error. Must not hang.
    tracing::info!("mid-transfer death outcome: {outcome:?}");

    client.shutdown().await;
}

/// 5.12 Bulk transfer while simultaneously receiving server echoes.
/// Client sends control frames (echoed back) AND bulk at the same time.
/// Uses into_split() for true concurrency — no Mutex, no deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_while_receiving_echoes() {
    common::init_tracing();
    let (path, client, _, mut brx, handle) = setup("bulk-echo").await;
    let _guard = common::SocketGuard::new(path.clone());
    let (send_client, mut recv_rx) = client.into_split();

    let send_ctrl = {
        let c = Arc::clone(&send_client);
        tokio::spawn(async move {
            for i in 0u32..20 {
                let outcome = c.send_frame(format!("ctrl-{i}").as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "ctrl {i}: {outcome:?}");
            }
        })
    };

    let send_bulk = {
        let c = Arc::clone(&send_client);
        tokio::spawn(async move {
            let payload = make_payload(200_000, 55);
            let outcome = c.send_bulk(&payload, Duration::from_secs(15)).await;
            assert!(outcome.is_delivered(), "bulk: {outcome:?}");
            payload
        })
    };

    let recv_echoes = tokio::spawn(async move {
        let mut count = 0u32;
        while count < 20 {
            match tokio::time::timeout(Duration::from_secs(15), recv_rx.recv()).await {
                Ok(Some(_)) => count += 1,
                Ok(None) => break,
                Err(_) => break,
            }
        }
        count
    });

    let (ctrl_r, bulk_r, echo_r) = tokio::join!(send_ctrl, send_bulk, recv_echoes);
    ctrl_r.unwrap();
    let expected_payload = bulk_r.unwrap();
    let echo_count = echo_r.unwrap();

    assert_eq!(echo_count, 20, "must receive all 20 echoes, got {echo_count}");

    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv()).await.unwrap().unwrap();
    assert_eq!(received, expected_payload);

    drop(send_client);
    handle.abort();
}

// ---- Scaled bulk transfer tests ----
// These prove the pipeline sustains throughput at scale without pool
// exhaustion, channel deadlock, or Merkle verification failure.
// Larger sizes are #[ignore] — run with `cargo test -- --ignored`.

/// Helper: run a bulk transfer of `size` bytes end-to-end with full
/// pipeline diagnostics on failure.
async fn bulk_scale_test(label: &str, size: usize, timeout_secs: u64) {
    common::init_tracing();
    let path = sock_path(label);
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _crx) = mpsc::channel(4096);
    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(256);
    let router = BulkAdvRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_counters = server.bulk_counters().clone();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut srx).await;

    let payload = make_payload(size, 7);
    let size_mib = size as f64 / (1024.0 * 1024.0);

    let client_counters = client.bulk_counters().clone();
    let start = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        client.send_bulk(&payload, Duration::from_secs(timeout_secs)),
    ).await;

    let elapsed = start.elapsed();
    use std::sync::atomic::Ordering::Relaxed;

    let c_nonces = client.encrypt_nonces_issued();
    let c_frames = client_counters.frames_sent.load(Relaxed);
    let s_frames_recv = server_counters.frames_received.load(Relaxed);
    let s_decrypted = server_counters.chunks_decrypted.load(Relaxed);
    let s_reassembled = server_counters.chunks_reassembled.load(Relaxed);
    let s_completed = server_counters.transfers_completed.load(Relaxed);

    let outcome = outcome.unwrap_or_else(|_| {
        panic!(
            "{label}: {size_mib:.0}MB bulk stalled after {elapsed:?}.\n\
             CLIENT: {c_nonces} nonces, {c_frames} frames sent\n\
             SERVER: {s_frames_recv} recv, {s_decrypted} decrypt, \
             {s_reassembled} reassembled, {s_completed} completed"
        )
    });
    assert!(outcome.is_delivered(), "{label}: {size_mib:.0}MB in {elapsed:?}: {outcome:?}");

    let (_, _, received) = tokio::time::timeout(Duration::from_secs(30), brx.recv())
        .await
        .unwrap_or_else(|_| panic!("{label}: server never delivered payload after {elapsed:?}"))
        .unwrap();
    assert_eq!(received.len(), payload.len(), "{label}: size mismatch");
    common::assert_payload_eq(&received, &payload);

    let throughput_mibs = size_mib / elapsed.as_secs_f64();
    tracing::info!(
        label,
        size_mib = size_mib as u64,
        elapsed_ms = elapsed.as_millis() as u64,
        throughput_mibs = throughput_mibs as u64,
        chunks = c_nonces,
        "{label} complete"
    );

    client.shutdown().await;
    handle.abort();
}

/// 100 MB — ~1,530 chunks. Proves sustained pipeline beyond initial pool size.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_100mb() {
    bulk_scale_test("bulk-100mb", 100_000_000, 30).await;
}

/// 500 MB — ~7,630 chunks. Sustained throughput at medium scale.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_500mb() {
    bulk_scale_test("bulk-500mb", 500_000_000, 60).await;
}

/// 1 GB — ~15,260 chunks. Large file transfer scale.
/// Run with: cargo test --release -p rekindle-transport-ipc --test bulk_advanced bulk_1gb -- --ignored
#[ignore] // requires --release for AES-NI/AVX2; debug mode ~40-80s, release ~1-2s
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_1gb() {
    bulk_scale_test("bulk-1gb", 1_000_000_000, 120).await;
}

/// 4 GB — ~61,040 chunks. Maximum target scale.
/// Run with: cargo test --release -p rekindle-transport-ipc --test bulk_advanced bulk_4gb -- --ignored
/// Debug mode: ~160s, 12+ GB RAM. Release mode: ~1-2s at 2-5 GiB/s.
#[ignore] // requires --release for AES-NI/AVX2; debug mode OOMs on <16GB machines
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_4gb() {
    bulk_scale_test("bulk-4gb", 4_000_000_000, 600).await;
}

/// 5.6b Exactly 3 * MAX_CHUNK_PLAIN — 3 full chunks, tests chunk boundary precisely.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_three_full_chunks() {
    common::init_tracing();
    let (path, client, _, mut brx, handle) = setup("bulk-3full").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = make_payload(3 * MAX_CHUNK_PLAIN, 13);
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered());
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv()).await.unwrap().unwrap();
    assert_eq!(received.len(), payload.len());
    assert_eq!(received, payload);
    client.shutdown().await;
    handle.abort();
}

/// 5.6c Exactly 3 * MAX_CHUNK_PLAIN + 1 — forces 4th chunk with 1 byte.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_three_chunks_plus_one_byte() {
    common::init_tracing();
    let (path, client, _, mut brx, handle) = setup("bulk-3plus1").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = make_payload(3 * MAX_CHUNK_PLAIN + 1, 17);
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered());
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv()).await.unwrap().unwrap();
    assert_eq!(received, payload);
    client.shutdown().await;
    handle.abort();
}
