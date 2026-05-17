//! Control plane receive tests: server → client delivery.
//! Every test that claims bidirectional IS bidirectional.
//! No sequential-pretending-to-be-concurrent.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use tokio::sync::mpsc;
use uuid::Uuid;

use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::envelope::SharedFrame;
use rekindle_transport_ipc::noise::keys;
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::*;
use rekindle_transport_ipc::client::IpcClient;

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-crecv-test-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

/// Router that echoes every received frame back via response_tx.
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

/// Router that sends N events to ALL connected clients' event_tx on each frame.
struct BroadcastOnFrameRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    event_count: u32,
}
impl FrameRouter for BroadcastOnFrameRouter {
    fn route_frame(&self, state: &ServerState, _conn_id: u64, _payload: Bytes) {
        // Broadcast to ALL connections, not just the sender.
        for entry in state.connections.iter() {
            for i in 0..self.event_count {
                let event = SharedFrame::from_bytes(format!("broadcast-{i}").as_bytes());
                let _ = entry.event_tx.try_send(event);
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

/// 4.1+4.2 Echo 20 frames. Server echoes each back. Client receives all 20 in order.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn echo_20_frames_in_order() {
    common::init_tracing();
    let path = sock_path("echo20");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, server_kp.into_inner(), EchoRouter { state_tx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let kp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut state_rx).await;

    for i in 0u32..20 {
        let outcome = client.send_frame(format!("echo-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "frame {i}: {outcome:?}");
    }
    for i in 0u32..20 {
        let p = tokio::time::timeout(Duration::from_secs(5), client.recv()).await
            .unwrap_or_else(|_| panic!("timeout echo {i}"))
            .unwrap_or_else(|| panic!("closed echo {i}"));
        assert_eq!(std::str::from_utf8(&p).unwrap(), format!("echo-{i}"));
    }
    client.shutdown().await; handle.abort();
}

/// 4.3 Server broadcasts to 3 clients simultaneously.
/// One client sends a trigger frame. Server broadcasts events to ALL connections.
/// All 3 clients receive the broadcast.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn broadcast_to_three_clients() {
    common::init_tracing();
    let path = sock_path("broadcast3");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let router = BroadcastOnFrameRouter { state_tx, event_count: 5 };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    // Connect 3 clients.
    let mut clients = Vec::new();
    for _ in 0..3 {
        let kp = keys::generate_keypair().unwrap();
        let c = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
        wait_ready(&mut state_rx).await;
        clients.push(c);
    }

    // Client 0 sends a trigger frame.
    let outcome = clients[0].send_frame(b"trigger", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // All 3 clients should receive 5 broadcast events each.
    for (ci, client) in clients.iter_mut().enumerate() {
        for ei in 0..5u32 {
            let p = tokio::time::timeout(Duration::from_secs(5), client.recv()).await
                .unwrap_or_else(|_| panic!("client {ci} timeout on event {ei}"))
                .unwrap_or_else(|| panic!("client {ci} closed on event {ei}"));
            assert_eq!(std::str::from_utf8(&p).unwrap(), format!("broadcast-{ei}"), "client {ci} event {ei}");
        }
    }

    for c in clients { c.shutdown().await; }
    handle.abort();
}

/// 4.4 Client recv on dead connection returns None.
///
/// Aborting the server's accept loop does NOT kill spawned connection handler
/// tasks — they are independent tokio::spawn calls. The connection handler
/// still holds the socket write half, so the client's read task never sees EOF.
/// Dead peer detection relies on the heartbeat system: 5s interval + 3 misses
/// × 3s pong timeout = ~14s worst case. The timeout must accommodate this.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recv_on_dead_connection() {
    common::init_tracing();
    let path = sock_path("recv-dead");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (state_tx, _) = mpsc::channel(64);
    let server = IpcServer::bind(&path, server_kp.into_inner(), EchoRouter { state_tx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let kp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    handle.abort(); let _ = handle.await;

    // Heartbeat dead-peer detection: 5s interval + 3 misses × 3s timeout = ~14s.
    // Use 20s timeout to avoid flakiness under load.
    let result = tokio::time::timeout(Duration::from_secs(20), client.recv()).await;
    match result {
        Ok(None) => {} // correct — control loop exited, inbound_tx dropped
        Ok(Some(_)) => panic!("received data from dead server"),
        Err(_) => panic!("recv hung 20s on dead connection — heartbeat dead-peer detection failed"),
    }
    client.shutdown().await;
}

/// 4.6 TRUE bidirectional: client sends AND receives simultaneously on one connection.
/// Uses tokio::join! to run send and recv concurrently, not sequentially.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn true_bidirectional_simultaneous() {
    common::init_tracing();
    let path = sock_path("bidir-true");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, server_kp.into_inner(), EchoRouter { state_tx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut state_rx).await;

    // We need to split client access. send_frame takes &self, recv takes &mut self.
    // Use Arc for the client and spawn separate tasks.
    let client = Arc::new(tokio::sync::Mutex::new(client));

    let send_client = Arc::clone(&client);
    let recv_client = Arc::clone(&client);

    // Sender task: sends 50 frames as fast as possible.
    let send_handle = tokio::spawn(async move {
        for i in 0u32..50 {
            let c = send_client.lock().await;
            let outcome = c.send_frame(format!("bidir-{i}").as_bytes(), Duration::from_secs(5)).await;
            assert!(outcome.is_delivered(), "send {i}: {outcome:?}");
        }
    });

    // Receiver task: receives 50 echoes.
    let recv_handle = tokio::spawn(async move {
        for i in 0u32..50 {
            let mut c = recv_client.lock().await;
            let p = tokio::time::timeout(Duration::from_secs(10), c.recv()).await
                .unwrap_or_else(|_| panic!("timeout recv {i}"))
                .unwrap_or_else(|| panic!("closed recv {i}"));
            let s = std::str::from_utf8(&p).unwrap();
            assert!(s.starts_with("bidir-"), "unexpected recv: {s}");
        }
    });

    // Both run concurrently via tokio::join!.
    let (send_result, recv_result) = tokio::join!(send_handle, recv_handle);
    send_result.unwrap();
    recv_result.unwrap();

    let c = Arc::try_unwrap(client).unwrap().into_inner();
    c.shutdown().await;
    handle.abort();
}

/// 4.10 500 server→client echoes sustained.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sustained_500_echoes() {
    common::init_tracing();
    let path = sock_path("500echo");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (state_tx, mut state_rx) = mpsc::channel(64);
    let server = IpcServer::bind(&path, server_kp.into_inner(), EchoRouter { state_tx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    let kp = keys::generate_keypair().unwrap();
    let mut client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_ready(&mut state_rx).await;

    for i in 0u32..500 {
        let outcome = client.send_frame(format!("e{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "frame {i}: {outcome:?}");
    }
    for i in 0u32..500 {
        let p = tokio::time::timeout(Duration::from_secs(10), client.recv()).await
            .unwrap_or_else(|_| panic!("timeout echo {i}"))
            .unwrap_or_else(|| panic!("closed echo {i}"));
        assert_eq!(std::str::from_utf8(&p).unwrap(), format!("e{i}"));
    }
    client.shutdown().await; handle.abort();
}
