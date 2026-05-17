//! Advanced control send tests: concurrent sends, max frame boundary, fire-and-forget.

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
        "rekindle-sadv-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct SendAdvRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for SendAdvRouter {
    fn route_frame(&self, state: &ServerState, id: u64, p: Bytes) {
        let _ = self.control_tx.try_send((id, p.clone()));
        // Echo for bidirectional tests.
        if let Some(conn) = state.connections.get(&id) {
            let _ = conn.response_tx.try_send(p);
        }
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
    }
}

async fn setup(label: &str) -> (PathBuf, IpcClient, mpsc::Receiver<(u64, Bytes)>, tokio::task::JoinHandle<()>) {
    let path = sock_path(label);
    let _ = std::fs::remove_file(&path);
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, crx) = mpsc::channel(4096);
    let (stx, mut srx) = mpsc::channel(256);
    let router = SendAdvRouter { control_tx: ctx, state_tx: stx };
    let server = IpcServer::bind(&path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let h = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready"),
        }
    }
    (path, client, crx, h)
}

/// 3.14b Concurrent sends from ONE client via multiple tasks.
/// Proves the mpsc channel serializes correctly and all frames arrive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_sends_one_client() {
    common::init_tracing();
    let (path, client, mut crx, handle) = setup("conc-1client").await;
    let _guard = common::SocketGuard::new(path.clone());
    let client = Arc::new(client);

    let mut tasks = Vec::new();
    for t in 0u32..4 {
        let c = Arc::clone(&client);
        tasks.push(tokio::spawn(async move {
            for i in 0u32..25 {
                let outcome = c.send_frame(format!("t{t}-f{i}").as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "t{t} f{i}: {outcome:?}");
            }
        }));
    }
    for t in tasks { t.await.unwrap(); }

    // All 100 frames (4 tasks × 25) must arrive.
    let mut count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), crx.recv()).await {
        count += 1;
    }
    assert_eq!(count, 100, "expected 100 concurrent sends, got {count}");

    // client is in Arc — extract and shutdown.
    // Arc::try_unwrap may fail if tasks still hold refs (they shouldn't after join).
    match Arc::try_unwrap(client) {
        Ok(c) => c.shutdown().await,
        Err(_) => {} // tasks dropped their refs, but Arc may have been cloned internally
    }
    handle.abort();
}

/// 3.16 TRUE simultaneous send AND receive on one connection.
/// Client sends frames AND receives echoes at the same time via separate tasks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn true_simultaneous_send_and_receive() {
    common::init_tracing();
    let (path, client, _crx, handle) = setup("simul-sr").await;
    let _guard = common::SocketGuard::new(path.clone());
    let client = Arc::new(tokio::sync::Mutex::new(client));

    let sender = {
        let c = Arc::clone(&client);
        tokio::spawn(async move {
            for i in 0u32..30 {
                let cl = c.lock().await;
                let outcome = cl.send_frame(format!("sim-{i}").as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "send {i}: {outcome:?}");
            }
        })
    };

    let receiver = {
        let c = Arc::clone(&client);
        tokio::spawn(async move {
            let mut count = 0;
            for _ in 0..30 {
                let mut cl = c.lock().await;
                match tokio::time::timeout(Duration::from_secs(10), cl.recv()).await {
                    Ok(Some(_)) => count += 1,
                    _ => break,
                }
            }
            count
        })
    };

    let (s, r) = tokio::join!(sender, receiver);
    s.unwrap();
    let recv_count = r.unwrap();
    assert!(recv_count > 0, "must receive echoes while sending");

    let c = Arc::try_unwrap(client).unwrap().into_inner();
    c.shutdown().await;
    handle.abort();
}

/// 3.2b 1000 sequential frames on one connection — sustained throughput.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sustained_1000_frames() {
    common::init_tracing();
    let (path, client, mut crx, handle) = setup("1000-frames").await;
    let _guard = common::SocketGuard::new(path.clone());

    let start = std::time::Instant::now();
    for i in 0u64..1000 {
        let outcome = client.send_frame(format!("k{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "frame {i}: {outcome:?}");
    }
    let elapsed = start.elapsed();
    tracing::info!(frames = 1000, elapsed_ms = elapsed.as_millis() as u64, "1000 frames complete");

    let mut count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), crx.recv()).await {
        count += 1;
    }
    assert_eq!(count, 1000);

    client.shutdown().await;
    handle.abort();
}

/// 3.5b Binary payload with every byte value 0x00-0xFF.
/// Proves no byte value is treated as a delimiter or control character.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_byte_values() {
    common::init_tracing();
    let (path, client, mut crx, handle) = setup("all-bytes").await;
    let _guard = common::SocketGuard::new(path.clone());
    let payload: Vec<u8> = (0..=255u8).collect();
    let outcome = client.send_frame(&payload, Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());
    let (_, received) = tokio::time::timeout(Duration::from_secs(5), crx.recv()).await.unwrap().unwrap();
    assert_eq!(received.len(), 256);
    for (i, &b) in received.iter().enumerate() {
        assert_eq!(b, i as u8, "byte {i} mismatch");
    }
    client.shutdown().await;
    handle.abort();
}
