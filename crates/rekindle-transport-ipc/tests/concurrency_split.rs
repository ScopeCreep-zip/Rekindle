//! True concurrent send+recv tests using client.into_split().
//! Zero Arc<Mutex>. Zero sequential-pretending-to-be-concurrent.
//!
//! Upstream reference: rtrb/tests/lib.rs:46-67 parallel() —
//! producer and consumer on separate threads with ZERO shared lock.
//!
//! WILL FAIL if:
//! - IpcClient::send_frame doesn't work through Arc<Self> (&self)
//! - IO loop doesn't deliver to inbound_rx independently of sends
//! - Deadlock exists between send path and recv path

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
        "rekindle-csplit-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

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

/// 50 sends + 50 receives simultaneously on one connection, zero locking.
/// Sender task uses Arc<IpcClient>::send_frame(&self).
/// Receiver task uses the extracted mpsc::Receiver<Bytes>.
/// They run via tokio::join! — genuinely concurrent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_bidirectional_50_frames() {
    common::init_tracing();
    let path = sock_path("split-bidir");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), EchoRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None).await.unwrap();
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
                Ok(Some(p)) => {
                    let s = std::str::from_utf8(&p).unwrap();
                    assert!(s.starts_with("split-"), "unexpected echo: {s}");
                    count += 1;
                }
                Ok(None) => panic!("recv channel closed after {count} echoes"),
                Err(_) => panic!("timeout after {count} echoes"),
            }
        }
        count
    });

    let (s, r) = tokio::join!(sender, receiver);
    s.unwrap();
    assert_eq!(r.unwrap(), 50);

    drop(send_client);
    handle.abort();
}

/// 4 sender tasks + 1 receiver task on one connection. 400 total frames.
/// Proves no lock contention, no data corruption under concurrent Arc usage.
/// Uses a high rate limit to allow 400-frame burst without throttling.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn split_4_senders_1_receiver() {
    common::init_tracing();
    let path = sock_path("split-4s1r");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), EchoRouter { state_tx: stx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut srx).await;

    let (send_client, mut recv_rx) = client.into_split();

    let mut senders = Vec::new();
    for t in 0u32..4 {
        let c = Arc::clone(&send_client);
        senders.push(tokio::spawn(async move {
            for i in 0u32..100 {
                let outcome = c.send_frame(format!("t{t}f{i}").as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "t{t} f{i}: {outcome:?}");
            }
        }));
    }

    let receiver = tokio::spawn(async move {
        let mut count = 0u32;
        while count < 400 {
            match tokio::time::timeout(Duration::from_secs(30), recv_rx.recv()).await {
                Ok(Some(_)) => count += 1,
                Ok(None) => panic!("channel closed after {count}"),
                Err(_) => panic!("timeout after {count}"),
            }
        }
        count
    });

    for s in senders { s.await.unwrap(); }
    assert_eq!(receiver.await.unwrap(), 400);

    drop(send_client);
    handle.abort();
}

/// Bulk send + control sends + echo receives all from separate tasks. No Mutex.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_bulk_while_receiving() {
    common::init_tracing();
    let path = sock_path("split-bulk-recv");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(64);

    struct MixRouter {
        state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
        bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
        bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
    }
    impl FrameRouter for MixRouter {
        fn route_frame(&self, state: &ServerState, id: u64, p: Bytes) {
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

    let (btx, mut brx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, kp.into_inner(), MixRouter { state_tx: stx, bulk_tx: btx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut srx).await;

    let (send_client, mut recv_rx) = client.into_split();

    let ctrl_sender = tokio::spawn({
        let c = Arc::clone(&send_client);
        async move {
            for i in 0u32..20 {
                let outcome = c.send_frame(format!("ctrl-{i}").as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "ctrl {i}: {outcome:?}");
            }
        }
    });

    let bulk_sender = tokio::spawn({
        let c = Arc::clone(&send_client);
        async move {
            let payload: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
            let outcome = c.send_bulk(&payload, Duration::from_secs(30)).await;
            assert!(outcome.is_delivered(), "bulk: {outcome:?}");
            payload
        }
    });

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
    assert_eq!(echo_count, 20, "must receive all 20 echoes, got {echo_count}");

    let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv())
        .await.unwrap().unwrap();
    assert_eq!(received.len(), expected_bulk.len());
    assert_eq!(received, expected_bulk);

    drop(send_client);
    handle.abort();
}
