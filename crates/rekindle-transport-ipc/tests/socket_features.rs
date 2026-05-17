//! Socket-level feature completeness tests.
//!
//! Every test binds a real Unix socket, performs a real Noise handshake,
//! and exercises a specific transport feature that node/cli/chat crates
//! depend on. Each test uses SocketGuard for panic-safe cleanup and
//! assert_payload_eq for diagnostic-rich failure messages.
//!
//! These tests prove the transport is usable as a library, not just
//! that individual components work in isolation.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
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

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-feat-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

// ---- Routers for specific feature tests ----

/// Router that echoes every frame back via response_tx.
struct EchoRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for EchoRouter {
    fn route_frame(&self, state: &ServerState, conn_id: u64, payload: Bytes) {
        if let Some(conn) = state.connections.get(&conn_id) {
            let _ = conn.response_tx.try_send(payload);
        }
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
    }
}

/// Router that pushes events to all connections on each frame.
struct EventPushRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    event_count: u32,
}
impl FrameRouter for EventPushRouter {
    fn route_frame(&self, state: &ServerState, conn_id: u64, _payload: Bytes) {
        // Broadcast events to all connections. Include the triggerer's conn_id
        // in the event payload so receivers can identify who initiated the broadcast.
        for entry in state.connections.iter() {
            for i in 0..self.event_count {
                let event = SharedFrame::from_bytes(
                    format!("event-{i}").as_bytes()
                );
                let _ = entry.event_tx.try_send(event);
            }
        }
        tracing::debug!(conn_id, "broadcast triggered");
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
    }
}

/// Router that tracks bulk completions and control frames.
struct FullRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
}
impl FrameRouter for FullRouter {
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

// ---- 1. Server-to-client response delivery (APP-tagged) ----

/// Proves: server echoes a frame back via response_tx and the client
/// receives it via recv(). The write_loop wraps it with TransportTag::APP
/// so the client IO loop delivers it to inbound_rx.
///
/// WILL FAIL if write_loop.rs doesn't prepend APP tag to responses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn response_delivery_app_tagged() {
    common::init_tracing();
    let path = sock_path("resp-app");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), EchoRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Send 5 frames, receive 5 echoes.
    for i in 0u32..5 {
        let outcome = client.send_frame(format!("echo-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "send {i}: {outcome:?}");
    }

    for i in 0u32..5 {
        let response = tokio::time::timeout(Duration::from_secs(5), client.recv())
            .await
            .unwrap_or_else(|_| panic!("timeout waiting for echo {i}"))
            .unwrap_or_else(|| panic!("channel closed waiting for echo {i}"));
        assert_eq!(
            std::str::from_utf8(&response).unwrap(),
            format!("echo-{i}"),
            "echo {i} content mismatch"
        );
    }

    client.shutdown().await;
    handle.abort();
}

// ---- 2. Server-to-client event delivery (APP-tagged) ----

/// Proves: server pushes events via event_tx and the client receives
/// them via recv(). Events are SharedFrame, written by the write_loop
/// with APP tag prepended.
///
/// WILL FAIL if write_loop.rs doesn't prepend APP tag to events.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn event_delivery_app_tagged() {
    common::init_tracing();
    let path = sock_path("event-app");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let router = EventPushRouter { state_tx: stx, event_count: 3 };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Trigger: send a frame, server pushes 3 events to all connections.
    let outcome = client.send_frame(b"trigger", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Receive 3 events.
    for i in 0u32..3 {
        let event = tokio::time::timeout(Duration::from_secs(5), client.recv())
            .await
            .unwrap_or_else(|_| panic!("timeout waiting for event {i}"))
            .unwrap_or_else(|| panic!("channel closed waiting for event {i}"));
        assert_eq!(
            std::str::from_utf8(&event).unwrap(),
            format!("event-{i}"),
            "event {i} content mismatch"
        );
    }

    client.shutdown().await;
    handle.abort();
}

// ---- 3. Response delivery with into_split (concurrent send+recv) ----

/// Proves: responses arrive while sends are in flight, using into_split
/// for true concurrency. No Mutex, no deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn response_delivery_split_concurrent() {
    common::init_tracing();
    let path = sock_path("resp-split");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), EchoRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    let (send_client, mut recv_rx) = client.into_split();

    let sender = tokio::spawn({
        let c = Arc::clone(&send_client);
        async move {
            for i in 0u32..50 {
                let outcome = c.send_frame(format!("split-{i}").as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "send {i}: {outcome:?}");
            }
        }
    });

    let receiver = tokio::spawn(async move {
        let mut count = 0u32;
        while count < 50 {
            match tokio::time::timeout(Duration::from_secs(10), recv_rx.recv()).await {
                Ok(Some(_)) => count += 1,
                Ok(None) => panic!("channel closed after {count} echoes"),
                Err(_) => panic!("timeout after {count} echoes"),
            }
        }
        count
    });

    let (s, r) = tokio::join!(sender, receiver);
    s.unwrap();
    assert_eq!(r.unwrap(), 50, "must receive all 50 echoes");

    drop(send_client);
    handle.abort();
}

// ---- 4. Typed message send_message API ----

/// Proves: send_message constructs a Message<T> envelope with the client's
/// identity, serializes it, and delivers it. The server decodes it and
/// verifies all envelope fields are correct.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_message_typed_roundtrip() {
    common::init_tracing();
    let path = sock_path("typed-send");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, mut crx) = mpsc::channel(64);
    let (btx, _) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = FullRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Use the high-level send_message API.
    let outcome = client.send_message(
        "typed-via-api".to_string(),
        SecurityLevel::Authenticated,
        Duration::from_secs(5),
    ).await;
    assert!(outcome.is_delivered(), "send_message: {outcome:?}");

    let (_, payload) = tokio::time::timeout(Duration::from_secs(5), crx.recv())
        .await.unwrap().unwrap();
    let decoded: Message<String> = decode_frame(&payload).unwrap();
    assert_eq!(decoded.payload, "typed-via-api");
    assert_eq!(decoded.sender, client.sender_id());
    assert_eq!(decoded.security_level, SecurityLevel::Authenticated);
    assert_eq!(decoded.wire_version, WIRE_VERSION);

    client.shutdown().await;
    handle.abort();
}

// ---- 5. send_message_with_correlation ----

/// Proves: correlation_id is set and preserved through the transport.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_message_with_correlation_roundtrip() {
    common::init_tracing();
    let path = sock_path("corr-msg");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, mut crx) = mpsc::channel(64);
    let (btx, _) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = FullRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    let corr_id = Uuid::now_v7();
    let outcome = client.send_message_with_correlation(
        "correlated-payload".to_string(),
        SecurityLevel::Open,
        corr_id,
        Duration::from_secs(5),
    ).await;
    assert!(outcome.is_delivered());

    let (_, payload) = tokio::time::timeout(Duration::from_secs(5), crx.recv())
        .await.unwrap().unwrap();
    let decoded: Message<String> = decode_frame(&payload).unwrap();
    assert_eq!(decoded.payload, "correlated-payload");
    assert_eq!(decoded.correlation_id, Some(corr_id));

    client.shutdown().await;
    handle.abort();
}

// ---- 6. send_message_to_community ----

/// Proves: community_scope is set and preserved through the transport.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_message_to_community_roundtrip() {
    common::init_tracing();
    let path = sock_path("comm-msg");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, mut crx) = mpsc::channel(64);
    let (btx, _) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = FullRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    let outcome = client.send_message_to_community(
        "community-payload".to_string(),
        SecurityLevel::Agent,
        "gov-key-abc123".to_string(),
        Duration::from_secs(5),
    ).await;
    assert!(outcome.is_delivered());

    let (_, payload) = tokio::time::timeout(Duration::from_secs(5), crx.recv())
        .await.unwrap().unwrap();
    let decoded: Message<String> = decode_frame(&payload).unwrap();
    assert_eq!(decoded.payload, "community-payload");
    assert_eq!(decoded.community_scope, Some("gov-key-abc123".to_string()));
    assert_eq!(decoded.security_level, SecurityLevel::Agent);

    client.shutdown().await;
    handle.abort();
}

// ---- 7. Bulk transfer with counter verification ----

/// Proves: after a bulk transfer completes, the server's BulkCounters
/// reflect the actual traffic. Both frames_received and bytes_received
/// increment correctly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_counters_after_transfer() {
    common::init_tracing();
    let path = sock_path("bulk-ctr");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _) = mpsc::channel(64);
    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = FullRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let counters = server.bulk_counters().clone();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    let frames_before = counters.frames_received.load(std::sync::atomic::Ordering::Relaxed);
    let bytes_before = counters.bytes_received.load(std::sync::atomic::Ordering::Relaxed);

    let payload: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
    let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered(), "bulk: {outcome:?}");

    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv())
        .await.unwrap().unwrap();
    common::assert_payload_eq(&received, &payload);

    let frames_after = counters.frames_received.load(std::sync::atomic::Ordering::Relaxed);
    let bytes_after = counters.bytes_received.load(std::sync::atomic::Ordering::Relaxed);

    assert!(frames_after > frames_before,
        "frames_received must increment: before={frames_before}, after={frames_after}");
    assert!(bytes_after > bytes_before,
        "bytes_received must increment: before={bytes_before}, after={bytes_after}");
    assert!(bytes_after - bytes_before >= payload.len() as u64,
        "bytes delta {} < payload size {}", bytes_after - bytes_before, payload.len());

    client.shutdown().await;
    handle.abort();
}

// ---- 8. Bulk + control interleaved on split handles ----

/// Proves: sending control frames AND bulk data simultaneously through
/// split handles. Both data paths work independently without interference.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_and_control_interleaved_split() {
    common::init_tracing();
    let path = sock_path("interleave");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, mut crx) = mpsc::channel(256);
    let (btx, mut brx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);

    struct InterleaveRouter {
        control_tx: mpsc::Sender<(u64, Bytes)>,
        bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
        bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
    }
    impl FrameRouter for InterleaveRouter {
        fn route_frame(&self, state: &ServerState, id: u64, p: Bytes) {
            let _ = self.control_tx.try_send((id, p.clone()));
            // Echo back for recv verification.
            if let Some(conn) = state.connections.get(&id) {
                let _ = conn.response_tx.try_send(p);
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

    let router = InterleaveRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    let (send_client, mut recv_rx) = client.into_split();

    // Sender 1: 20 control frames.
    let ctrl_sender = tokio::spawn({
        let c = Arc::clone(&send_client);
        async move {
            for i in 0u32..20 {
                let outcome = c.send_frame(format!("ctrl-{i}").as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "ctrl {i}: {outcome:?}");
            }
        }
    });

    // Sender 2: 200KB bulk.
    let bulk_sender = tokio::spawn({
        let c = Arc::clone(&send_client);
        async move {
            let payload: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
            let outcome = c.send_bulk(&payload, Duration::from_secs(15)).await;
            assert!(outcome.is_delivered(), "bulk: {outcome:?}");
            payload
        }
    });

    // Receiver: echoes from control frames.
    let echo_recv = tokio::spawn(async move {
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

    let (c, b, r) = tokio::join!(ctrl_sender, bulk_sender, echo_recv);
    c.unwrap();
    let expected_bulk = b.unwrap();
    let echo_count = r.unwrap();
    assert_eq!(echo_count, 20, "must receive all 20 echoes during bulk, got {echo_count}");

    // Verify all 20 control frames arrived at server.
    let mut ctrl_count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), crx.recv()).await {
        ctrl_count += 1;
    }
    assert_eq!(ctrl_count, 20, "server must receive all 20 control frames");

    // Verify bulk arrived.
    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv())
        .await.unwrap().unwrap();
    common::assert_payload_eq(&received, &expected_bulk);

    drop(send_client);
    handle.abort();
}

// ---- 9. msg_ctx accessor returns correct identity ----

/// Proves: client.msg_ctx() returns the MessageContext with the correct
/// sender_id, matching what was passed to connect().
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn msg_ctx_returns_correct_identity() {
    common::init_tracing();
    let path = sock_path("msgctx");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, _) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), EchoRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let sender_id = Uuid::now_v7();
    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        sender_id, &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    assert_eq!(client.msg_ctx().sender, sender_id);
    assert_eq!(client.sender_id(), sender_id);

    client.shutdown().await;
    handle.abort();
}

// ---- 10. Multiple clients with events to all ----

/// Proves: when the server pushes events to ALL connections, every
/// connected client receives them. This is the foundation for
/// subscription-based event routing in chat/node.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn broadcast_events_to_multiple_clients() {
    common::init_tracing();
    let path = sock_path("broadcast-evt");
    let _guard = common::SocketGuard::new(path.clone());
    let _ = std::fs::remove_file(&path);

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let router = EventPushRouter { state_tx: stx, event_count: 3 };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    // Connect 3 clients.
    let mut clients = Vec::new();
    for _ in 0..3 {
        let ckp = keys::generate_keypair().unwrap();
        let c = IpcClient::connect(
            Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
        ).await.unwrap();
        wait_ready(&mut srx).await;
        clients.push(c);
    }

    // Client 0 sends trigger — server pushes 3 events to ALL connections.
    let outcome = clients[0].send_frame(b"trigger-broadcast", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Each client should receive 3 events.
    for (ci, client) in clients.iter_mut().enumerate() {
        for ei in 0u32..3 {
            let event = tokio::time::timeout(Duration::from_secs(5), client.recv())
                .await
                .unwrap_or_else(|_| panic!("client {ci} timeout on event {ei}"))
                .unwrap_or_else(|| panic!("client {ci} channel closed on event {ei}"));
            assert_eq!(
                std::str::from_utf8(&event).unwrap(),
                format!("event-{ei}"),
                "client {ci} event {ei}"
            );
        }
    }

    for c in clients { c.shutdown().await; }
    handle.abort();
}
