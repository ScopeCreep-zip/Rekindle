//! max_connections enforcement tests.
//!
//! Upstream reference: governor/governor/tests/direct.rs:16-37 rejects_too_many
//!
//! WILL FAIL if:
//! - Server accept loop (server/mod.rs:144-197) doesn't check max_connections
//! - Connection counter doesn't decrement on disconnect
//! - Rejected client hangs instead of failing
//!
//! NOTE: As of the current implementation, the server accept loop at
//! server/mod.rs:144-197 does NOT check config.max_connections. The
//! config field exists (config.rs:38) and is validated (config.rs:103-105)
//! but is never consulted during accept(). These tests WILL FAIL until
//! the implementation adds enforcement. That is the point.

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
        "rekindle-maxconn-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct MaxConnRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for MaxConnRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
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

/// Set max_connections=2. Connect 2 clients. Attempt 3rd — must be rejected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn max_connections_enforced() {
    common::init_tracing();
    let path = sock_path("maxconn-enforce");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);

    let mut config = IpcConfig::default();
    config.max_connections = 2;

    let router = MaxConnRouter { control_tx: ctx, state_tx: stx };
    let server = IpcServer::bind(&path, kp.into_inner(), router, config.clone()).unwrap();
    let ss = server.state();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    // Client 1.
    let kp1 = keys::generate_keypair().unwrap();
    let c1 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp1.as_inner(), &config, None).await.unwrap();
    wait_ready(&mut srx).await;
    assert_eq!(ss.connections.len(), 1);

    // Client 2.
    let kp2 = keys::generate_keypair().unwrap();
    let c2 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp2.as_inner(), &config, None).await.unwrap();
    wait_ready(&mut srx).await;
    assert_eq!(ss.connections.len(), 2);

    // Client 3: must be rejected (connect fails or handshake times out).
    let kp3 = keys::generate_keypair().unwrap();
    let mut reject_config = config.clone();
    reject_config.handshake_timeout_ms = 2000;
    let c3_result = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, kp3.as_inner(), &reject_config, None,
    ).await;

    match c3_result {
        Err(_) => {
            // Connection rejected — correct.
        }
        Ok(c3) => {
            // Connected — check it doesn't reach Ready (server should reject at max).
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    match srx.try_recv() {
                        Ok((_, _, ConnectionPhase::Ready)) => return true,
                        _ => {}
                    }
                    tokio::task::yield_now().await;
                }
            }).await;

            assert!(
                result.is_err(),
                "third client must NOT reach Ready when max_connections=2. \
                 Server accept loop at server/mod.rs:144-197 does not check \
                 config.max_connections — this must be implemented."
            );
            drop(c3);
        }
    }

    assert!(
        ss.connections.len() <= 2,
        "server exceeded max_connections: have {} (max=2)",
        ss.connections.len()
    );

    c1.shutdown().await;
    c2.shutdown().await;
    handle.abort();
}

/// After hitting max, disconnect one client, new client can connect.
/// Proves the counter decrements on disconnect.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn max_connections_decrements_on_disconnect() {
    common::init_tracing();
    let path = sock_path("maxconn-decrement");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(256);

    let mut config = IpcConfig::default();
    config.max_connections = 2;
    config.drain_timeout_ms = 500; // Short drain so cleanup completes within test deadline.

    let router = MaxConnRouter { control_tx: ctx, state_tx: stx };
    let server = IpcServer::bind(&path, kp.into_inner(), router, config.clone()).unwrap();
    let ss = server.state();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    // Fill to max.
    let kp1 = keys::generate_keypair().unwrap();
    let c1 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp1.as_inner(), &config, None).await.unwrap();
    let id1 = wait_ready(&mut srx).await;

    let kp2 = keys::generate_keypair().unwrap();
    let c2 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp2.as_inner(), &config, None).await.unwrap();
    wait_ready(&mut srx).await;
    assert_eq!(ss.connections.len(), 2);

    // Disconnect c1.
    c1.shutdown().await;

    // Wait for cleanup.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((id, _, phase))) if id == id1 && phase.is_terminal() => break,
            Ok(Some(_)) => continue,
            _ => panic!("c1 never reached terminal"),
        }
    }
    ss.wait_connection_count(1, Duration::from_secs(2)).await;

    // New client should connect now.
    let kp3 = keys::generate_keypair().unwrap();
    let c3 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp3.as_inner(), &config, None).await.unwrap();
    wait_ready(&mut srx).await;

    let outcome = c3.send_frame(b"after-slot-freed", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered(), "new client after slot freed: {outcome:?}");

    c2.shutdown().await;
    c3.shutdown().await;
    handle.abort();
}
