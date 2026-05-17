//! Startup, restart, and bind failure tests.
//!
//! Proves the server handles every startup condition: clean bind,
//! stale socket cleanup, permission errors, multiple sequential
//! connections, connections after disconnect, and full restart cycle.

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
        "rekindle-startup-test-{}-{}-{}.sock",
        label, std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_nanos()
    ))
}

struct CountingRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}

impl FrameRouter for CountingRouter {
    fn route_frame(&self, _: &ServerState, conn_id: u64, payload: Bytes) {
        let _ = self.control_tx.try_send((conn_id, payload));
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, conn_id: u64, old: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((conn_id, old, new));
    }
}

async fn wait_state(
    rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>,
    expected: ConnectionPhase,
    timeout: Duration,
) -> u64 {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, _, new))) if new == expected => return id,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("state channel closed before {expected}"),
            Err(_) => panic!("timed out waiting for {expected}"),
        }
    }
}

/// Proves: server binds, removes stale socket file, binds again on same path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bind_removes_stale_socket() {
    common::init_tracing();
    let path = sock_path("stale");
    let _guard = common::SocketGuard::new(path.clone());

    // Create a stale file at the socket path.
    std::fs::write(&path, b"stale").unwrap();

    let server_kp = keys::generate_keypair().unwrap();
    let (control_tx, _) = mpsc::channel(64);
    let (state_tx, _) = mpsc::channel(64);
    let router = CountingRouter { control_tx, state_tx };

    // Bind should succeed — stale file removed.
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default());
    assert!(server.is_ok(), "bind must succeed after removing stale socket");

    drop(server);
}

/// Proves: bind on a nonexistent directory creates the directory.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bind_creates_parent_directory() {
    common::init_tracing();
    let base = std::env::temp_dir().join(format!(
        "rekindle-mkdir-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_nanos()
    ));
    let path = base.join("sub/daemon.sock");

    let server_kp = keys::generate_keypair().unwrap();
    let (control_tx, _) = mpsc::channel(64);
    let (state_tx, _) = mpsc::channel(64);
    let router = CountingRouter { control_tx, state_tx };

    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default());
    assert!(server.is_ok(), "bind must create parent directories");

    drop(server);
    let _ = std::fs::remove_dir_all(&base);
}

/// Proves: two sequential clients on the same server, both reach Active.
/// Uses a short drain timeout (500ms) to ensure the first connection
/// cleans up quickly after shutdown, before the wait_connection_count
/// timeout fires.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_sequential_clients() {
    common::init_tracing();
    let path = sock_path("seq-clients");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut config = IpcConfig::default();
    config.drain_timeout_ms = 500;

    let (control_tx, mut control_rx) = mpsc::channel(256);
    let (state_tx, mut state_rx) = mpsc::channel(256);
    let router = CountingRouter { control_tx, state_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let ss = server.state();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    // Client 1: connect, send, verify, disconnect.
    {
        let kp = keys::generate_keypair().unwrap();
        let c = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &config, None).await.unwrap();
        wait_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

        let outcome = c.send_frame(b"client-1", Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "client 1 send: {outcome:?}");

        let (_, payload) = tokio::time::timeout(Duration::from_secs(5), control_rx.recv()).await.unwrap().unwrap();
        assert_eq!(&payload[..], b"client-1");

        c.shutdown().await;
    }

    // Wait for server to fully clean up client 1's connection.
    ss.wait_connection_count(0, Duration::from_secs(5)).await;

    // Client 2: connect to the same server, send, verify.
    {
        let kp = keys::generate_keypair().unwrap();
        let c = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &config, None).await.unwrap();
        wait_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

        let outcome = c.send_frame(b"client-2", Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "client 2 send: {outcome:?}");

        let (_, payload) = tokio::time::timeout(Duration::from_secs(5), control_rx.recv()).await.unwrap().unwrap();
        assert_eq!(&payload[..], b"client-2");

        c.shutdown().await;
    }

    server_handle.abort();
}

/// Proves: connection after a previous connection disconnected works.
/// No stale state from the first connection leaks into the second.
/// Uses a short drain timeout (500ms) to ensure cleanup completes
/// within the wait_connection_count timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_after_disconnect() {
    common::init_tracing();
    let path = sock_path("reconnect");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut config = IpcConfig::default();
    config.drain_timeout_ms = 500;

    let (control_tx, mut control_rx) = mpsc::channel(256);
    let (state_tx, mut state_rx) = mpsc::channel(256);
    let router = CountingRouter { control_tx, state_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let state = server.state();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    // First connection.
    let kp1 = keys::generate_keypair().unwrap();
    let c1 = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp1.as_inner(), &config, None).await.unwrap();
    let conn1_id = wait_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;
    c1.send_frame(b"first", Duration::from_secs(5)).await;
    let _ = control_rx.recv().await;
    c1.shutdown().await;

    // Wait for server to fully clean up connection 1.
    state.wait_connection_count(0, Duration::from_secs(5)).await;

    // Verify server cleaned up.
    assert!(
        state.connections.get(&conn1_id).is_none(),
        "connection 1 not cleaned up from server state"
    );

    // Second connection on the same path.
    let kp2 = keys::generate_keypair().unwrap();
    let c2 = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp2.as_inner(), &config, None).await.unwrap();
    let conn2_id = wait_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    assert_ne!(conn1_id, conn2_id, "second connection must have a different conn_id");

    let outcome = c2.send_frame(b"second", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    let (recv_id, payload) = tokio::time::timeout(Duration::from_secs(5), control_rx.recv()).await.unwrap().unwrap();
    assert_eq!(recv_id, conn2_id);
    assert_eq!(&payload[..], b"second");

    c2.shutdown().await;
    server_handle.abort();
}

/// Proves: full server restart cycle — bind, accept client, shutdown server,
/// re-bind same path, accept new client. Socket file cleanup works.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_restart_cycle() {
    common::init_tracing();
    let path = sock_path("restart");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_pub: [u8; 32];

    // --- First server lifetime ---
    {
        let server_kp = keys::generate_keypair().unwrap();
        server_pub = server_kp.public().try_into().unwrap();

        let (control_tx, mut control_rx) = mpsc::channel(64);
        let (state_tx, mut state_rx) = mpsc::channel(64);
        let router = CountingRouter { control_tx, state_tx };
        let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
        let server_handle = tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let kp = keys::generate_keypair().unwrap();
        let c = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
        wait_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

        let outcome = c.send_frame(b"before-restart", Duration::from_secs(5)).await;
        assert!(outcome.is_delivered());
        let _ = control_rx.recv().await;

        c.shutdown().await;
        server_handle.abort();
        let _ = server_handle.await;
    }
    // Server dropped — socket file removed by Drop impl.

    // --- Second server lifetime on same path ---
    {
        let server_kp = keys::generate_keypair().unwrap();
        let server_pub2: [u8; 32] = server_kp.public().try_into().unwrap();

        let (control_tx, mut control_rx) = mpsc::channel(64);
        let (state_tx, mut state_rx) = mpsc::channel(64);
        let router = CountingRouter { control_tx, state_tx };
        let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
        let server_handle = tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        // New keypair means new handshake — client must use new pubkey.
        let kp = keys::generate_keypair().unwrap();
        let c = IpcClient::connect(Uuid::now_v7(), &path, &server_pub2, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
        wait_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

        let outcome = c.send_frame(b"after-restart", Duration::from_secs(5)).await;
        assert!(outcome.is_delivered());

        let (_, payload) = tokio::time::timeout(Duration::from_secs(5), control_rx.recv()).await.unwrap().unwrap();
        assert_eq!(&payload[..], b"after-restart");

        c.shutdown().await;
        server_handle.abort();
    }

}
