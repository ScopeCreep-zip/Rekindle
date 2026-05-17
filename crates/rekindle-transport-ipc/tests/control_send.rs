//! Control plane send tests: every send condition, every edge case.
//! Every test asserts a SPECIFIC outcome. No test accepts "either outcome is fine."

mod common;

use std::path::PathBuf;
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
        "rekindle-csend-test-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct SendRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for SendRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, old: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, old, new));
    }
}

async fn setup(label: &str) -> (PathBuf, IpcClient, mpsc::Receiver<(u64, Bytes)>, tokio::task::JoinHandle<()>) {
    let path = sock_path(label);
    let _ = std::fs::remove_file(&path);
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (control_tx, control_rx) = mpsc::channel(4096);
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let router = SendRouter { control_tx, state_tx };
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
    (path, client, control_rx, handle)
}

/// 3.4 Send 0-byte payload — succeeds with empty delivery.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_empty_payload() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("empty").await;
    let _guard = common::SocketGuard::new(path.clone());
    let outcome = client.send_frame(b"", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered(), "empty: {outcome:?}");
    let (_, p) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(p.is_empty());
    client.shutdown().await; handle.abort();
}

/// 3.5 Send 1-byte payload.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_one_byte() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("1byte").await;
    let _guard = common::SocketGuard::new(path.clone());
    let outcome = client.send_frame(&[0xAA], Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());
    let (_, p) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert_eq!(&p[..], &[0xAA]);
    client.shutdown().await; handle.abort();
}

/// 3.6 Send at exact Noise chunk boundary (65519 bytes = MAX_NOISE_PLAINTEXT).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_exact_noise_chunk() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("noise-exact").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = vec![0xCC; 65519]; // exactly one Noise chunk
    let outcome = client.send_frame(&payload, Duration::from_secs(10)).await;
    assert!(outcome.is_delivered());
    let (_, p) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    assert_eq!(p.len(), 65519);
    assert!(p.iter().all(|&b| b == 0xCC));
    client.shutdown().await; handle.abort();
}

/// 3.6b Send 65520 bytes (one byte over Noise chunk, forces 2 chunks).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_one_over_noise_chunk() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("noise-plus1").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = vec![0xDD; 65520];
    let outcome = client.send_frame(&payload, Duration::from_secs(10)).await;
    assert!(outcome.is_delivered());
    let (_, p) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    assert_eq!(p.len(), 65520);
    client.shutdown().await; handle.abort();
}

/// 3.8 Send 1 MiB — exercises multi-chunk Noise transport.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_1mib() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("1mib").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload = vec![0xAB; 1_048_576];
    let outcome = client.send_frame(&payload, Duration::from_secs(15)).await;
    assert!(outcome.is_delivered());
    let (_, p) = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await.unwrap().unwrap();
    assert_eq!(p.len(), 1_048_576);
    assert_eq!(&p[..], &payload[..]);
    client.shutdown().await; handle.abort();
}

/// 3.10 Send after server death — MUST return non-Delivered, MUST NOT hang.
///
/// Aborting the accept loop does NOT kill already-spawned connection handler
/// tasks — they are independent tokio::spawn calls. The IpcServer::Drop fires
/// cancel_token.cancel() which signals all connection handlers to shut down.
/// We must wait for the connection handler to fully exit (drain timeout +
/// socket close) before the client's IO loop detects the dead peer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_after_server_death() {
    common::init_tracing();
    let path = sock_path("dead-send");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut config = IpcConfig::default();
    config.drain_timeout_ms = 200; // fast cleanup

    let (control_tx, _rx) = mpsc::channel(4096);
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let router = SendRouter { control_tx, state_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let ss = server.state();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &config, None).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, state_rx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never reached Ready"),
        }
    }

    // Drop the server — cancel_token fires, connection handler begins teardown.
    handle.abort();
    let _ = handle.await;

    // Wait for the connection handler to fully exit (drain_timeout 200ms + margin).
    ss.wait_connection_count(0, Duration::from_secs(3)).await;

    // Give the client IO loop time to process the EOF.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let outcome = client.send_frame(b"should-fail", Duration::from_secs(3)).await;
    assert!(!outcome.is_delivered(), "send to dead server must not return Delivered: {outcome:?}");
    client.shutdown().await;
}

/// 3.15 Ack seq consistency: 10 sequential frames, all acked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ack_seq_10_frames() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("ack10").await;
    let _guard = common::SocketGuard::new(path.clone());
    for i in 0u32..10 {
        let outcome = client.send_frame(format!("seq-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "frame {i}: {outcome:?}");
    }
    for i in 0u32..10 {
        let (_, p) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert_eq!(std::str::from_utf8(&p).unwrap(), format!("seq-{i}"));
    }
    client.shutdown().await; handle.abort();
}

/// Varying payload sizes in sequence — proves no framing desync.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn varying_sizes_no_desync() {
    common::init_tracing();
    let (path, client, mut rx, handle) = setup("vary").await;
    let _guard = common::SocketGuard::new(path.clone());
    let sizes = [1, 10, 100, 1000, 10_000, 65_519, 65_520, 100_000, 1, 0, 50_000];
    for (i, &size) in sizes.iter().enumerate() {
        let payload = vec![(i as u8).wrapping_add(1); size];
        let outcome = client.send_frame(&payload, Duration::from_secs(10)).await;
        assert!(outcome.is_delivered(), "size {size}: {outcome:?}");
    }
    for (i, &size) in sizes.iter().enumerate() {
        let (_, p) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        assert_eq!(p.len(), size, "frame {i} size");
        if size > 0 {
            assert!(p.iter().all(|&b| b == (i as u8).wrapping_add(1)), "frame {i} content");
        }
    }
    client.shutdown().await; handle.abort();
}

/// 3.9 Typed Message<T> roundtrip preserves all fields.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn typed_message_roundtrip() {
    common::init_tracing();
    use rekindle_transport_ipc::envelope::*;
    use rekindle_transport_ipc::frame::codec::{encode_frame, decode_frame};
    let (path, client, mut rx, handle) = setup("typed").await;
    let _guard = common::SocketGuard::new(path.clone());
    let ctx = MessageContext::new(client.sender_id());
    let msg = Message::new(&ctx, "typed payload".to_string(), SecurityLevel::Open, client.epoch());
    let encoded = encode_frame(&msg).unwrap();
    let outcome = client.send_frame(&encoded, Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());
    let (_, p) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    let decoded: Message<String> = decode_frame(&p).unwrap();
    assert_eq!(decoded.payload, "typed payload");
    assert_eq!(decoded.sender, client.sender_id());
    assert_eq!(decoded.wire_version, WIRE_VERSION);
    client.shutdown().await; handle.abort();
}
