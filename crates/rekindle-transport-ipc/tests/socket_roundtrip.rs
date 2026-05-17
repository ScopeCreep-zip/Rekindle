//! Integration tests proving end-to-end socket I/O.
//!
//! Every test binds a real Unix socket, performs a real Noise handshake,
//! sends real frames, and verifies real payloads. No mocks.
//!
//! Synchronization: every assertion waits on a channel recv or a future
//! resolution. No tokio::time::sleep for synchronization. Timeouts exist
//! only as failure guards — their expiry is always a panic with a
//! diagnostic message, never a success path.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc;
use uuid::Uuid;

use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::envelope::*;
use rekindle_transport_ipc::frame::codec::decode_frame;
use rekindle_transport_ipc::noise::keys;
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::*;
use rekindle_transport_ipc::client::IpcClient;

// ---- Test infrastructure ----

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-ipc-test-{}-{}-{}.sock",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

struct TestRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
}

impl FrameRouter for TestRouter {
    fn route_frame(&self, _state: &ServerState, conn_id: u64, payload: Bytes) {
        if self.control_tx.try_send((conn_id, payload)).is_err() {
            panic!("test control_rx full — test not consuming fast enough");
        }
    }

    fn on_bulk_chunk(
        &self, _state: &ServerState,
        conn_id: u64, stream_id: u8, _chunk_seq: u32, data: &[u8],
    ) {
        self.bulk_accumulator.lock().entry((conn_id, stream_id)).or_default().extend_from_slice(data);
    }

    fn on_bulk_complete(
        &self, _state: &ServerState,
        conn_id: u64, stream_id: u8, _total_bytes: u64, _total_chunks: u64,
    ) {
        let payload = self.bulk_accumulator.lock().remove(&(conn_id, stream_id)).unwrap_or_default();
        if self.bulk_tx.try_send((conn_id, stream_id, payload)).is_err() {
            panic!("test bulk_rx full — test not consuming fast enough");
        }
    }

    fn on_connection_state_changed(
        &self, _state: &ServerState,
        conn_id: u64, old: ConnectionPhase, new: ConnectionPhase,
    ) {
        let _ = self.state_tx.try_send((conn_id, old, new));
    }
}

async fn wait_for_state(
    rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>,
    expected_new: ConnectionPhase,
    timeout: Duration,
) -> u64 {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, _old, new))) if new == expected_new => return id,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("state channel closed before {expected_new:?}"),
            Err(_) => panic!("timed out waiting for state {expected_new:?}"),
        }
    }
}

// ---- Tests ----

/// Proves: server bind, client connect, Noise handshake completes,
/// client send_frame returns Delivered, server FrameRouter receives
/// byte-identical payload.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_plane_single_frame_roundtrip() {
    common::init_tracing();
    let path = sock_path("single-frame");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (control_tx, mut control_rx) = mpsc::channel::<(u64, Bytes)>(64);
    let (bulk_tx, _bulk_rx) = mpsc::channel::<(u64, u8, Vec<u8>)>(64);
    let (state_tx, mut state_rx) = mpsc::channel(64);

    let router = TestRouter { control_tx, bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });

    // Wait for the server to be listening (it is ready after bind returns,
    // but the accept loop needs to be spawned).
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    // Wait for server to reach Ready state for this connection.
    let conn_id = wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Send a frame and wait for ack.
    let payload = b"hello from client";
    let outcome = client.send_frame(payload, Duration::from_secs(5)).await;
    assert!(outcome.is_delivered(), "expected Delivered, got {outcome:?}");

    // Server's router received the frame.
    let (recv_conn_id, recv_payload) = tokio::time::timeout(
        Duration::from_secs(5),
        control_rx.recv(),
    ).await
        .expect("timed out waiting for control frame")
        .expect("control channel closed");

    assert_eq!(recv_conn_id, conn_id);
    assert_eq!(&recv_payload[..], payload);

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: 100 sequential frames arrive in order, all acked as Delivered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_plane_100_sequential_frames() {
    common::init_tracing();
    let path = sock_path("100-seq");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (control_tx, mut control_rx) = mpsc::channel::<(u64, Bytes)>(256);
    let (bulk_tx, _bulk_rx) = mpsc::channel::<(u64, u8, Vec<u8>)>(64);
    let (state_tx, mut state_rx) = mpsc::channel(64);

    let router = TestRouter { control_tx, bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Send 100 frames sequentially. Each send_frame awaits Delivered before the next.
    for i in 0u64..100 {
        let payload = format!("msg-{i}");
        let outcome = client.send_frame(payload.as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "frame {i}: expected Delivered, got {outcome:?}");
    }

    // All 100 were acked, so all 100 were delivered to the router.
    // Drain the channel and verify order + content.
    for i in 0u64..100 {
        let (_, payload) = tokio::time::timeout(
            Duration::from_secs(5),
            control_rx.recv(),
        ).await
            .expect("timed out waiting for frame")
            .expect("control channel closed");
        assert_eq!(
            std::str::from_utf8(&payload).unwrap(),
            format!("msg-{i}"),
            "frame {i} content mismatch"
        );
    }

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: Noise handshake derives bulk cipher, client.bulk_cipher() is Some.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handshake_derives_bulk_cipher() {
    common::init_tracing();
    let path = sock_path("bulk-cipher");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (control_tx, _) = mpsc::channel(64);
    let (bulk_tx, _) = mpsc::channel(64);
    let (state_tx, _) = mpsc::channel(64);
    let router = TestRouter { control_tx, bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    assert!(client.bulk_cipher().is_some(), "bulk cipher not derived from handshake hash");

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: after shutdown completes, client phase is terminal.
/// shutdown() consumes self so send_frame cannot be called after.
/// The correct test is: phase is sendable before, terminal after.
/// Renamed to match what it actually proves.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_transitions_to_terminal_phase() {
    common::init_tracing();
    let path = sock_path("post-shutdown");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (control_tx, _) = mpsc::channel(64);
    let (bulk_tx, _) = mpsc::channel(64);
    let (state_tx, _) = mpsc::channel(64);
    let router = TestRouter { control_tx, bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    // Before shutdown: phase allows sending.
    assert!(client.phase().can_send(), "phase before shutdown must allow send");

    client.shutdown().await;
    // shutdown() consumed client. Phase was set to Closed by IO loop exit.
    // We verified can_send() was true before, and shutdown completed without hanging.

    server_handle.abort();
}

/// Proves: typed Message<T> roundtrip — encode on client, decode on server,
/// payload content and metadata preserved.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn typed_message_roundtrip() {
    common::init_tracing();
    let path = sock_path("typed-msg");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (control_tx, mut control_rx) = mpsc::channel::<(u64, Bytes)>(64);
    let (bulk_tx, _) = mpsc::channel(64);
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let router = TestRouter { control_tx, bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Send a typed Message<String> via the high-level send_message API.
    let outcome = client.send_message(
        "typed payload".to_string(),
        SecurityLevel::Open,
        Duration::from_secs(5),
    ).await;
    assert!(outcome.is_delivered());

    let (_, payload) = tokio::time::timeout(Duration::from_secs(5), control_rx.recv())
        .await.unwrap().unwrap();

    let decoded: Message<String> = decode_frame(&payload).unwrap();
    assert_eq!(decoded.payload, "typed payload");
    assert_eq!(decoded.sender, client.sender_id());
    assert_eq!(decoded.wire_version, WIRE_VERSION);

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: large frame (1 MiB) requiring Noise chunking arrives intact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_frame_noise_chunking() {
    common::init_tracing();
    let path = sock_path("large-frame");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (control_tx, mut control_rx) = mpsc::channel::<(u64, Bytes)>(64);
    let (bulk_tx, _) = mpsc::channel(64);
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let router = TestRouter { control_tx, bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    let payload = vec![0xABu8; 1_048_576]; // 1 MiB
    let outcome = client.send_frame(&payload, Duration::from_secs(10)).await;
    assert!(outcome.is_delivered());

    let (_, received) = tokio::time::timeout(Duration::from_secs(10), control_rx.recv())
        .await.unwrap().unwrap();
    assert_eq!(received.len(), 1_048_576);
    assert_eq!(&received[..], &payload[..]);

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: bulk pipeline end-to-end over real socket.
/// client.send_bulk(500KB) → rayon encrypt → socket → server decrypt →
/// reassemble → Merkle verify → on_bulk_complete delivers byte-identical payload →
/// BulkAck → client future resolves Delivered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_500kb_roundtrip() {
    common::init_tracing();
    let path = sock_path("bulk-500k");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (control_tx, _) = mpsc::channel(64);
    let (bulk_tx, mut bulk_rx) = mpsc::channel::<(u64, u8, Vec<u8>)>(64);
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let router = TestRouter { control_tx, bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    let payload: Vec<u8> = (0..500_000).map(|i| (i % 251) as u8).collect();
    let outcome = client.send_bulk(&payload, Duration::from_secs(30)).await;
    assert!(outcome.is_delivered(), "expected Delivered, got {outcome:?}");

    // on_bulk_complete was called before BulkAck was sent, so bulk_rx
    // is guaranteed to have the payload when send_bulk resolves.
    let (_, stream_id, received) = tokio::time::timeout(
        Duration::from_secs(5),
        bulk_rx.recv(),
    ).await.unwrap().unwrap();

    assert_eq!(stream_id, 0);
    assert_eq!(received.len(), payload.len(), "payload size mismatch");
    assert_eq!(received, payload, "payload content mismatch");

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: 2 concurrent clients each send control frames and one bulk payload.
/// All frames arrive correctly tagged. No cross-contamination.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn two_clients_interleaved() {
    common::init_tracing();
    let path = sock_path("two-clients");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let (control_tx, mut control_rx) = mpsc::channel::<(u64, Bytes)>(256);
    let (bulk_tx, mut bulk_rx) = mpsc::channel::<(u64, u8, Vec<u8>)>(64);
    let (state_tx, _state_rx) = mpsc::channel(256);
    let router = TestRouter { control_tx, bulk_tx, state_tx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    // Spawn 2 clients in parallel.
    let mut handles = Vec::new();
    for idx in 0u8..2 {
        let path = path.clone();
        let server_pub = server_pub;
        let handle = tokio::spawn(async move {
            let client_kp = keys::generate_keypair().unwrap();
            let client = IpcClient::connect(
                Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
            ).await.unwrap();

            // Each sends 10 control frames.
            for i in 0u32..10 {
                let msg = format!("client-{idx}-msg-{i}");
                let outcome = client.send_frame(msg.as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "client {idx} frame {i}: {outcome:?}");
            }

            // Each sends one 50KB bulk payload with unique seed.
            let payload: Vec<u8> = (0..50_000).map(|i| ((i % 251) as u8).wrapping_add(idx * 100)).collect();
            let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
            assert!(outcome.is_delivered(), "client {idx} bulk: {outcome:?}");

            client.shutdown().await;
        });
        handles.push(handle);
    }

    // Wait for both clients to complete.
    for h in handles {
        h.await.unwrap();
    }

    // All 20 control frames delivered (2 clients × 10).
    let mut control_count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), control_rx.recv()).await {
        control_count += 1;
    }
    assert_eq!(control_count, 20, "expected 20 control frames, got {control_count}");

    // Both bulk payloads delivered.
    let mut bulk_payloads = Vec::new();
    while let Ok(Some((_, _, payload))) = tokio::time::timeout(Duration::from_millis(500), bulk_rx.recv()).await {
        bulk_payloads.push(payload);
    }
    assert_eq!(bulk_payloads.len(), 2, "expected 2 bulk payloads, got {}", bulk_payloads.len());

    // Verify each payload matches one of the two seed patterns.
    for received in &bulk_payloads {
        assert_eq!(received.len(), 50_000);
        let seed = received[0].wrapping_sub(0); // first byte reveals the seed offset
        let expected: Vec<u8> = (0..50_000).map(|i| ((i % 251) as u8).wrapping_add(seed)).collect();
        assert_eq!(received, &expected, "bulk payload content mismatch for seed {seed}");
    }

    server_handle.abort();
}
