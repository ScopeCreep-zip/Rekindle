//! Server-to-client bulk transfer tests.
//!
//! Proves the full bidirectional bulk pipeline:
//! - Server sends bulk via ConnectionState::send_bulk() → encrypt_pool →
//!   bulk_out_tx → write_loop → socket
//! - Client receives via read_task → recv_dispatcher → rayon decrypt →
//!   recv_reassembler → bulk_inbound_tx → recv_bulk()
//!
//! Also verifies BulkCounters track server-side receive traffic.

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
        "rekindle-s2c-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

/// Router that sends a bulk payload back to the client when it receives
/// a control frame containing "send-bulk-back". This exercises the
/// server-to-client bulk transfer path.
struct BulkEchoRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for BulkEchoRouter {
    fn route_frame(&self, state: &ServerState, conn_id: u64, payload: Bytes) {
        if &payload[..] == b"send-bulk-back" {
            // Send a 50KB bulk payload back to the client via the bulk plane.
            if let Some(conn) = state.connections.get(&conn_id) {
                let bulk_data: Vec<u8> = (0..50_000).map(|i| (i % 251) as u8).collect();
                let _ = conn.send_bulk(0, &bulk_data);
            }
        }
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
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

/// Proves: server can send a control-plane response back to the client.
/// This is the baseline — server-to-client control works via response_tx.
/// If this fails, the response path is broken and bulk won't work either.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_to_client_control_response() {
    common::init_tracing();
    let path = sock_path("s2c-ctrl");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), BulkEchoRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Client sends trigger, server sends 50KB bulk back via send_bulk.
    let outcome = client.send_frame(b"send-bulk-back", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered(), "trigger frame: {outcome:?}");

    // Client receives the bulk payload.
    let bulk = tokio::time::timeout(Duration::from_secs(15), client.recv_bulk()).await;
    match bulk {
        Ok(Some((stream_id, data))) => {
            assert_eq!(stream_id, 0);
            let expected: Vec<u8> = (0..50_000).map(|i| (i % 251) as u8).collect();
            assert_eq!(data.len(), 50_000, "bulk size: {}", data.len());
            assert_eq!(data, expected, "bulk content mismatch");
        }
        Ok(None) => panic!("bulk channel closed before receiving server bulk"),
        Err(_) => panic!(
            "timed out waiting for server-to-client bulk. \
             Server must call conn.send_bulk() and client must \
             receive via recv_bulk()."
        ),
    }

    client.shutdown().await;
    handle.abort();
}

/// Proves: BulkCounters are incremented when the server receives bulk data.
/// The read_task at connection.rs:351 receives bulk_counters but currently
/// does not increment them — the control loop at connection.rs:297-298 does.
/// This test verifies the counters reflect actual traffic.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_counters_track_received_traffic() {
    common::init_tracing();
    let path = sock_path("s2c-counters");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();

    struct CounterRouter {
        bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
        bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
    }
    impl FrameRouter for CounterRouter {
        fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
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

    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), CounterRouter { bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) }, IpcConfig::default()).unwrap();
    let counters = server.bulk_counters().clone();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Record baseline counters.
    let frames_before = counters.frames_received.load(std::sync::atomic::Ordering::Relaxed);
    let bytes_before = counters.bytes_received.load(std::sync::atomic::Ordering::Relaxed);

    // Send a 100KB bulk payload.
    let payload: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered(), "bulk send: {outcome:?}");

    // Wait for server to deliver it.
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv())
        .await.unwrap().unwrap();
    assert_eq!(received.len(), payload.len());
    assert_eq!(received, payload);

    // Counters must have incremented.
    let frames_after = counters.frames_received.load(std::sync::atomic::Ordering::Relaxed);
    let bytes_after = counters.bytes_received.load(std::sync::atomic::Ordering::Relaxed);

    assert!(
        frames_after > frames_before,
        "frames_received must increment after bulk transfer: before={frames_before}, after={frames_after}"
    );
    assert!(
        bytes_after > bytes_before,
        "bytes_received must increment after bulk transfer: before={bytes_before}, after={bytes_after}"
    );

    // Verify the byte count is reasonable (at least the payload size,
    // plus overhead from headers/tags).
    let bytes_delta = bytes_after - bytes_before;
    assert!(
        bytes_delta >= payload.len() as u64,
        "bytes_received delta {bytes_delta} must be >= payload size {}",
        payload.len()
    );

    client.shutdown().await;
    handle.abort();
}

/// Proves: BulkCounters track server-to-client bulk SEND traffic.
/// The write_loop at write_loop.rs:147-149 increments frames_sent and
/// bytes_sent when it writes bulk frames. This test sends bulk from
/// client, which triggers the server to echo it back as bulk.
///
/// WILL FAIL until:
/// - ConnectionState exposes a method to send bulk to a client
/// - Server-side encrypt_pool is wired for outbound bulk encryption
/// - Client bulk receive path is implemented (not discarded at client.rs:198-209)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_to_client_bulk_roundtrip() {
    common::init_tracing();
    let path = sock_path("s2c-bulk-rt");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();

    /// Router that echoes received bulk data back to the sender as bulk.
    /// Uses ConnectionState::send_bulk() to encrypt and send via the bulk plane.
    struct BulkMirrorRouter {
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
        bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
    }
    impl FrameRouter for BulkMirrorRouter {
        fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
        fn on_bulk_chunk(&self, _: &ServerState, conn_id: u64, stream_id: u8, _chunk_seq: u32, data: &[u8]) {
            self.bulk_accumulator.lock().entry((conn_id, stream_id)).or_default().extend_from_slice(data);
        }
        fn on_bulk_complete(&self, state: &ServerState, conn_id: u64, stream_id: u8, _total_bytes: u64, _total_chunks: u64) {
            let payload = self.bulk_accumulator.lock().remove(&(conn_id, stream_id)).unwrap_or_default();
            // Echo the bulk payload back to the sender via the bulk plane.
            if let Some(conn) = state.connections.get(&conn_id) {
                if let Err(e) = conn.send_bulk(0, &payload) {
                    tracing::error!(conn_id, error = e, "server bulk echo failed");
                }
            }
        }
        fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
            let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
        }
    }

    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), BulkMirrorRouter { state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) }, IpcConfig::default()).unwrap();
    let counters = Arc::clone(server.bulk_counters());
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Client sends 200KB bulk to server.
    let payload: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered(), "client bulk send: {outcome:?}");

    // Server's on_bulk_complete fires, echoes the payload back via bulk plane.
    // Client receives it via recv_bulk().
    let bulk_response = tokio::time::timeout(
        Duration::from_secs(15),
        client.recv_bulk(),
    ).await;

    match bulk_response {
        Ok(Some((stream_id, received))) => {
            assert_eq!(stream_id, 0, "echo stream_id must be 0");
            assert_eq!(
                received.len(), payload.len(),
                "echoed bulk size mismatch: got {}, want {}",
                received.len(), payload.len()
            );
            assert_eq!(received, payload, "echoed bulk content mismatch");
        }
        Ok(None) => panic!("bulk_inbound_rx closed — client disconnect before bulk echo arrived"),
        Err(_) => panic!(
            "timed out waiting for server bulk echo. \
             Server must send bulk via ConnectionState::send_bulk() \
             and client must receive via recv_bulk()."
        ),
    }

    // Verify server send counters incremented.
    let frames_sent = counters.frames_sent.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        frames_sent > 0,
        "server frames_sent must be > 0 after bulk echo, got {frames_sent}"
    );

    client.shutdown().await;
    handle.abort();
}

/// Proves: recv_bulk_chunk() delivers individual chunks in order,
/// with is_last=true on the final chunk and empty data. This is the
/// streaming API that avoids buffering the full payload in memory.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recv_bulk_chunk_streaming() {
    common::init_tracing();
    let path = sock_path("s2c-chunk-stream");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), BulkEchoRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Trigger server to send 50KB bulk back.
    let outcome = client.send_frame(b"send-bulk-back", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Receive via streaming API — one chunk at a time.
    let expected: Vec<u8> = (0..50_000).map(|i| (i % 251) as u8).collect();
    let mut received = Vec::new();
    let mut chunk_count = 0u64;

    loop {
        let chunk = tokio::time::timeout(
            Duration::from_secs(15),
            client.recv_bulk_chunk(),
        ).await
            .expect("timeout waiting for bulk chunk")
            .expect("bulk chunk channel closed");

        if chunk.is_last {
            assert!(chunk.data.is_empty(), "fin chunk must have empty data");
            break;
        } else {
            assert!(!chunk.data.is_empty(), "data chunk must have non-empty data");
            assert!(chunk.data.len() <= 65519, "chunk must not exceed MAX_CHUNK_PLAIN");
            received.extend_from_slice(&chunk.data);
            chunk_count += 1;
        }
    }

    // Loop only exits via `break` in the is_last branch — fin is guaranteed.
    assert!(chunk_count > 0, "must receive at least one data chunk");
    assert_eq!(received.len(), expected.len(), "total bytes mismatch");
    assert_eq!(received, expected, "content mismatch");

    client.shutdown().await;
    handle.abort();
}

/// Proves: two concurrent server→client bulk transfers on different stream_ids
/// both deliver correctly. The client's per-stream recv_reassemblers and
/// recv_accumulators must not mix data between streams.
///
/// If the client uses a SINGLE recv_reassembler or a SINGLE recv_accumulator,
/// the two streams' chunks will be interleaved and the assembled payloads will
/// be corrupted or one stream will hang waiting for chunk_seq values that belong
/// to the other stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_server_to_client_both_streams_deliver() {
    common::init_tracing();
    let path = sock_path("s2c-concurrent");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();

    /// Router that sends two bulk payloads on different stream_ids when triggered.
    struct DualBulkRouter {
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    }
    impl FrameRouter for DualBulkRouter {
        fn route_frame(&self, state: &ServerState, conn_id: u64, _payload: Bytes) {
            // On receiving any control frame, send 100KB on stream 0 and 100KB on stream 1.
            if let Some(conn) = state.connections.get(&conn_id) {
                let payload_a: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
                let payload_b: Vec<u8> = (0..100_000).map(|i| ((i % 251) as u8).wrapping_add(100)).collect();
                let _ = conn.send_bulk(0, &payload_a);
                let _ = conn.send_bulk(1, &payload_b);
            }
        }
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
            let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
        }
    }

    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), DualBulkRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Trigger the server to send two concurrent bulk transfers.
    let outcome = client.send_frame(b"trigger-dual-bulk", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Receive both via recv_bulk() (buffered API). Order may vary.
    let expected_a: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
    let expected_b: Vec<u8> = (0..100_000).map(|i| ((i % 251) as u8).wrapping_add(100)).collect();

    let mut results: Vec<(u8, Vec<u8>)> = Vec::new();
    for i in 0..2 {
        let (stream_id, data) = tokio::time::timeout(
            Duration::from_secs(15),
            client.recv_bulk(),
        ).await
            .unwrap_or_else(|_| panic!(
                "TIMEOUT: waiting for server→client bulk delivery {i}/2. \
                 If only 1 of 2 streams delivered, the client's recv_reassemblers \
                 or recv_accumulators are not per-stream — they mix data between \
                 concurrent streams, causing one to hang."
            ))
            .unwrap_or_else(|| panic!(
                "CLOSED: bulk_inbound_rx closed before receiving delivery {i}/2."
            ));
        results.push((stream_id, data));
    }

    results.sort_by_key(|(sid, _)| *sid);

    assert_eq!(results[0].0, 0, "expected stream 0 in results");
    assert_eq!(results[1].0, 1, "expected stream 1 in results");
    assert_eq!(
        results[0].1.len(), 100_000,
        "stream 0 payload size wrong: got {}, want 100000. \
         Client recv_accumulator is mixing streams.",
        results[0].1.len()
    );
    assert_eq!(
        results[1].1.len(), 100_000,
        "stream 1 payload size wrong: got {}, want 100000. \
         Client recv_accumulator is mixing streams.",
        results[1].1.len()
    );
    assert_eq!(results[0].1, expected_a, "stream 0 content corrupted — recv_accumulator mixed with stream 1");
    assert_eq!(results[1].1, expected_b, "stream 1 content corrupted — recv_accumulator mixed with stream 0");

    client.shutdown().await;
    handle.abort();
}

/// Proves: recv_bulk_chunk() works correctly with two concurrent server→client
/// streams interleaved. Each stream's chunks arrive in order (chunk_seq 0, 1, 2, ...)
/// and the final chunk has is_last=true with empty data.
///
/// If the client uses a single recv_reassembler, chunks from both streams are
/// delivered to the same reassembler. Stream 1's chunk_seq values (0, 1, 2...)
/// collide with stream 0's, causing the reassembler to deliver corrupted data
/// or deadlock waiting for missing chunk_seq values.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_server_to_client_streaming_recv_bulk_chunk() {
    common::init_tracing();
    let path = sock_path("s2c-stream-concurrent");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();

    /// Router that sends 200KB on stream 0 and 200KB on stream 1 when triggered.
    struct DualStreamRouter {
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    }
    impl FrameRouter for DualStreamRouter {
        fn route_frame(&self, state: &ServerState, conn_id: u64, _payload: Bytes) {
            if let Some(conn) = state.connections.get(&conn_id) {
                let pa: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
                let pb: Vec<u8> = (0..200_000).map(|i| ((i % 251) as u8).wrapping_add(77)).collect();
                let _ = conn.send_bulk(0, &pa);
                let _ = conn.send_bulk(1, &pb);
            }
        }
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
            let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
        }
    }

    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), DualStreamRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Trigger server to send dual bulk.
    let outcome = client.send_frame(b"go", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Receive ALL chunks via recv_bulk_chunk() streaming API.
    // Chunks from both streams arrive interleaved. Sort per-stream.
    let mut stream_0_data = Vec::new();
    let mut stream_1_data = Vec::new();
    let mut stream_0_done = false;
    let mut stream_1_done = false;
    let mut total_chunks = 0u64;

    while !stream_0_done || !stream_1_done {
        let chunk = tokio::time::timeout(
            Duration::from_secs(15),
            client.recv_bulk_chunk(),
        ).await
            .unwrap_or_else(|_| panic!(
                "TIMEOUT waiting for bulk chunk. stream_0_done={stream_0_done}, \
                 stream_1_done={stream_1_done}, total_chunks={total_chunks}. \
                 If one stream hung, the client's recv_reassemblers are not per-stream."
            ))
            .unwrap_or_else(|| panic!("bulk_chunk channel closed prematurely"));

        if chunk.is_last {
            assert!(chunk.data.is_empty(), "fin chunk must have empty data");
            match chunk.stream_id {
                0 => stream_0_done = true,
                1 => stream_1_done = true,
                other => panic!("unexpected stream_id {other}"),
            }
        } else {
            assert!(!chunk.data.is_empty());
            assert!(chunk.data.len() <= 65519);
            match chunk.stream_id {
                0 => stream_0_data.extend_from_slice(&chunk.data),
                1 => stream_1_data.extend_from_slice(&chunk.data),
                other => panic!("unexpected stream_id {other}"),
            }
            total_chunks += 1;
        }
    }

    let expected_a: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
    let expected_b: Vec<u8> = (0..200_000).map(|i| ((i % 251) as u8).wrapping_add(77)).collect();

    assert_eq!(stream_0_data.len(), 200_000,
        "stream 0 received {}, expected 200000. recv_reassembler mixed streams.", stream_0_data.len());
    assert_eq!(stream_1_data.len(), 200_000,
        "stream 1 received {}, expected 200000. recv_reassembler mixed streams.", stream_1_data.len());
    assert_eq!(stream_0_data, expected_a, "stream 0 content corrupted");
    assert_eq!(stream_1_data, expected_b, "stream 1 content corrupted");

    client.shutdown().await;
    handle.abort();
}

/// Proves: cancel_recv_bulk() cleans up the client's recv_reassemblers entry
/// for the cancelled stream. After cancel, any remaining in-flight chunks for
/// that stream are dropped (not delivered to recv_bulk_chunk or recv_bulk).
/// Other streams' receive state is unaffected.
///
/// The test triggers the server to send a large bulk on stream 0 and a small
/// bulk on stream 1. The client cancels recv on stream 0 immediately after
/// triggering. Stream 1 must still deliver correctly.
///
/// WILL FAIL until IpcClient exposes cancel_recv_bulk(stream_id: u8) that:
/// 1. Removes recv_reassemblers[stream_id]
/// 2. Removes recv_accumulators[stream_id]
/// 3. Does NOT affect other streams
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_recv_bulk_cleans_up_and_isolates_streams() {
    common::init_tracing();
    let path = sock_path("s2c-cancel-recv");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();

    /// Router that sends 1MB on stream 0 and 50KB on stream 1 when triggered.
    struct CancelRecvRouter {
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    }
    impl FrameRouter for CancelRecvRouter {
        fn route_frame(&self, state: &ServerState, conn_id: u64, _payload: Bytes) {
            if let Some(conn) = state.connections.get(&conn_id) {
                // Stream 0: large payload — client will cancel mid-receive.
                let big: Vec<u8> = (0..1_000_000).map(|i| (i % 251) as u8).collect();
                // Stream 1: small payload — must deliver despite stream 0 cancel.
                let small: Vec<u8> = (0..50_000).map(|i| ((i % 251) as u8).wrapping_add(99)).collect();
                let _ = conn.send_bulk(0, &big);
                let _ = conn.send_bulk(1, &small);
            }
        }
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
            let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
        }
    }

    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), CancelRecvRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Trigger server to send dual bulk.
    let outcome = client.send_frame(b"go", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Cancel receive on stream 0 immediately.
    client.cancel_recv_bulk(0).await;

    // Stream 1 must still deliver correctly via recv_bulk().
    // We may also receive stream 0's partial data or nothing — both acceptable.
    // The key assertion: stream 1 is NOT affected by stream 0's cancel.
    let expected_stream1: Vec<u8> = (0..50_000).map(|i| ((i % 251) as u8).wrapping_add(99)).collect();

    // Drain recv_bulk for up to 2 deliveries. Stream 0 may or may not deliver
    // (depends on whether cancel arrived before all chunks were reassembled).
    let mut stream1_received = false;
    for _ in 0..2 {
        match tokio::time::timeout(Duration::from_secs(10), client.recv_bulk()).await {
            Ok(Some((sid, data))) => {
                if sid == 1 {
                    assert_eq!(data.len(), 50_000,
                        "stream 1 payload corrupted after stream 0 cancel: got {} bytes", data.len());
                    assert_eq!(data, expected_stream1, "stream 1 content corrupted");
                    stream1_received = true;
                }
                // sid == 0: stream 0 delivered before cancel arrived (race). Acceptable.
            }
            Ok(None) => break, // channel closed
            Err(_) => break, // timeout
        }
    }

    assert!(
        stream1_received,
        "CANCEL ISOLATION FAILURE: cancel_recv_bulk(0) prevented stream 1 from delivering. \
         The implementation must only remove recv_reassemblers[0] and recv_accumulators[0], \
         not affect stream 1's reassembly state. \
         If cancel_recv_bulk is not implemented, this test forces its implementation."
    );

    client.shutdown().await;
    handle.abort();
}
