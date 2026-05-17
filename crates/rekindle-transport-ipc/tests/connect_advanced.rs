//! Advanced connect/handshake tests: stress, edge cases.

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
        "rekindle-cadv-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct ConnAdvRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    control_tx: mpsc::Sender<(u64, Bytes)>,
}
impl FrameRouter for ConnAdvRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
    }
}

async fn make_server(path: &std::path::Path) -> ([u8; 32], mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>, mpsc::Receiver<(u64, Bytes)>, tokio::task::JoinHandle<()>) {
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, srx) = mpsc::channel(1024);
    let (ctx, crx) = mpsc::channel(1024);
    let router = ConnAdvRouter { state_tx: stx, control_tx: ctx };
    let server = IpcServer::bind(path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let h = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    (pub_key, srx, crx, h)
}

/// 2.13b 50 clients connect in rapid succession — all succeed and can send.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn fifty_rapid_connects_all_send() {
    common::init_tracing();
    let path = sock_path("50-rapid");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut srx, mut crx, handle) = make_server(&path).await;

    let mut handles = Vec::new();
    for idx in 0u32..50 {
        let path = path.clone();
        handles.push(tokio::spawn(async move {
            let kp = keys::generate_keypair().unwrap();
            let c = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
            let outcome = c.send_frame(format!("rapid-{idx}").as_bytes(), Duration::from_secs(10)).await;
            assert!(outcome.is_delivered(), "client {idx}: {outcome:?}");
            c.shutdown().await;
        }));
    }
    for h in handles { h.await.unwrap(); }

    // Count Ready transitions and delivered frames.
    let mut ready_count = 0;
    while let Ok(Some((_, _, new))) = tokio::time::timeout(Duration::from_millis(500), srx.recv()).await {
        if new == ConnectionPhase::Ready { ready_count += 1; }
    }
    assert_eq!(ready_count, 50, "all 50 must reach Ready, got {ready_count}");

    let mut frame_count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), crx.recv()).await {
        frame_count += 1;
    }
    assert_eq!(frame_count, 50, "all 50 frames must arrive, got {frame_count}");

    handle.abort();
}

/// 2.8b Server survives client that sends corrupted msg1 content.
/// (Properly length-framed but garbage Noise handshake content.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corrupted_msg1_server_survives() {
    common::init_tracing();
    let path = sock_path("bad-msg1");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut srx, mut crx, handle) = make_server(&path).await;

    // Send a properly framed garbage msg1.
    {
        use tokio::io::AsyncWriteExt;
        let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        let garbage = vec![0xDE; 48]; // Noise IK msg1 is 48 bytes but this is random
        let len = (garbage.len() as u32).to_le_bytes();
        stream.write_all(&len).await.unwrap();
        stream.write_all(&garbage).await.unwrap();
        stream.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(stream);
    }

    // Legitimate client after.
    let kp = keys::generate_keypair().unwrap();
    let c = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready after bad msg1"),
        }
    }
    let outcome = c.send_frame(b"after-bad-msg1", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Verify the frame was actually delivered to the router, not just acked.
    let (_, payload) = tokio::time::timeout(Duration::from_secs(5), crx.recv())
        .await.expect("frame never delivered to router after bad msg1")
        .expect("control channel closed");
    assert_eq!(&payload[..], b"after-bad-msg1");

    c.shutdown().await;
    handle.abort();
}

/// 2.6b Server-side handshake timeout: client connects via raw socket,
/// sends the correct length-prefix for msg1 but never sends the body.
/// Server must timeout and continue accepting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_side_handshake_timeout() {
    common::init_tracing();
    let path = sock_path("srv-hs-timeout");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut srx, _, handle) = make_server(&path).await;

    // Connect and send only a partial handshake (length prefix, no body).
    {
        use tokio::io::AsyncWriteExt;
        let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        // Send length prefix declaring 48 bytes but never send the body.
        let len = 48u32.to_le_bytes();
        stream.write_all(&len).await.unwrap();
        stream.flush().await.unwrap();
        // Hold connection for 2 seconds, then drop.
        tokio::time::sleep(Duration::from_secs(2)).await;
        drop(stream);
    }

    // Server must still accept legitimate clients.
    let kp = keys::generate_keypair().unwrap();
    let c = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready after server-side handshake timeout"),
        }
    }
    let outcome = c.send_frame(b"after-srv-timeout", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());
    c.shutdown().await;
    handle.abort();
}
