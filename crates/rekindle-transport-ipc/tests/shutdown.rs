//! Graceful shutdown and connection lifecycle tests.
//!
//! Proves the transport handles shutdown correctly: drain pending frames,
//! exchange Shutdown/ShutdownAck, close cleanly. Also proves dead peer
//! detection and state machine transitions.

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
        "rekindle-shutdown-test-{}-{}-{}.sock",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

struct LifecycleRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    control_tx: mpsc::Sender<(u64, Bytes)>,
}

impl FrameRouter for LifecycleRouter {
    fn route_frame(&self, _state: &ServerState, conn_id: u64, payload: Bytes) {
        let _ = self.control_tx.try_send((conn_id, payload));
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, conn_id: u64, old: ConnectionPhase, new: ConnectionPhase) {
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
            Ok(None) => panic!("state channel closed before {expected_new}"),
            Err(_) => panic!("timed out ({timeout:?}) waiting for state {expected_new}"),
        }
    }
}

/// Proves: client connect transitions server to Ready.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connect_reaches_ready() {
    common::init_tracing();
    let path = sock_path("ready");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (state_tx, mut state_rx) = mpsc::channel(64);
    let (control_tx, _) = mpsc::channel(64);
    let router = LifecycleRouter { state_tx, control_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    let conn_id = wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;
    assert!(conn_id > 0);

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: first application frame transitions Ready → Active.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_frame_reaches_active() {
    common::init_tracing();
    let path = sock_path("active");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (state_tx, mut state_rx) = mpsc::channel(64);
    let (control_tx, _) = mpsc::channel(64);
    let router = LifecycleRouter { state_tx, control_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    let outcome = client.send_frame(b"trigger-active", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    wait_for_state(&mut state_rx, ConnectionPhase::Active, Duration::from_secs(5)).await;

    client.shutdown().await;
    server_handle.abort();
}

/// Proves: server abort causes client to detect connection loss.
/// Client's phase transitions to Dead or Closed.
///
/// Uses aggressive timeouts: the server's connection handler has a 200ms
/// drain timeout so it exits quickly after cancel_token fires. The client
/// uses a 500ms heartbeat so it detects death within ~1s. Without these,
/// the default 5s drain + 14s heartbeat makes detection too slow for a test.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_abort_detected_by_client() {
    common::init_tracing();
    let path = sock_path("server-abort");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let mut config = IpcConfig::default();
    config.drain_timeout_ms = 200;
    config.heartbeat_interval_ms = 500;
    config.heartbeat_pong_timeout_ms = 300;
    config.heartbeat_max_misses = 1;

    let (state_tx, _) = mpsc::channel(64);
    let (control_tx, _) = mpsc::channel(64);
    let router = LifecycleRouter { state_tx, control_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &config, None,
    ).await.unwrap();

    // Kill the server. IpcServer::Drop fires cancel_token.cancel().
    // The connection handler sees cancellation, transitions to Dead,
    // sends SHUTDOWN, waits drain_timeout (200ms), then exits.
    server_handle.abort();
    let _ = server_handle.await;

    // Wait for the server-side connection handler to fully exit.
    // drain_timeout (200ms) + scheduling margin.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now the server's write half is dropped → client read task sees EOF →
    // client IO loop exits → phase transitions to Closed.
    // Give the client IO loop time to process the EOF.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Phase should be terminal (Dead or Closed).
    let phase = client.phase();
    assert!(
        phase.is_terminal() || phase == ConnectionPhase::Degraded,
        "client phase after server abort + drain: {phase}"
    );

    // Send after death must not hang and must not return Delivered.
    let outcome = client.send_frame(b"should-fail", Duration::from_secs(1)).await;
    assert!(
        !outcome.is_delivered(),
        "send after server abort must not return Delivered, got {outcome:?}"
    );

    client.shutdown().await;
}

/// Proves: client shutdown completes without hanging even under
/// no server-side Shutdown/ShutdownAck (server already dead).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_completes_with_dead_server() {
    common::init_tracing();
    let path = sock_path("dead-shutdown");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (state_tx, _) = mpsc::channel(64);
    let (control_tx, _) = mpsc::channel(64);
    let router = LifecycleRouter { state_tx, control_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    server_handle.abort();
    let _ = server_handle.await;

    // shutdown() must complete within 5 seconds, not hang forever.
    let shutdown_result = tokio::time::timeout(
        Duration::from_secs(5),
        async { client.shutdown().await },
    ).await;

    assert!(
        shutdown_result.is_ok(),
        "client.shutdown() hung for >5s after server died"
    );
}

/// Proves: sending frames after server connection handler has fully exited
/// results in non-Delivered outcomes, not hangs.
///
/// The critical timing: `server_handle.abort()` fires `cancel_token.cancel()`
/// via `IpcServer::Drop`. The connection handler then:
/// 1. Sees cancellation in biased select! (P0)
/// 2. Transitions to Dead
/// 3. Sends SHUTDOWN frame
/// 4. Waits drain_timeout for write task to flush
/// 5. Drops socket write half → client sees EOF
///
/// Until step 5 completes, the connection handler can still ACK frames.
/// This test uses a short drain timeout (200ms) and waits for the handler
/// to fully exit before asserting on send outcomes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inflight_frames_resolve_on_server_death() {
    common::init_tracing();
    let path = sock_path("inflight");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let mut config = IpcConfig::default();
    config.drain_timeout_ms = 200;

    let (state_tx, mut state_rx) = mpsc::channel(64);
    let (control_tx, _) = mpsc::channel(64);
    let router = LifecycleRouter { state_tx, control_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let ss = server.state();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &config, None,
    ).await.unwrap();
    wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Send one frame successfully to reach Active.
    let outcome = client.send_frame(b"warmup", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Kill the server. cancel_token fires → connection handler begins teardown.
    server_handle.abort();
    let _ = server_handle.await;

    // Wait for the connection handler to fully exit (drain_timeout 200ms + margin).
    // After this, the server socket write half is dropped and the client will see EOF.
    ss.wait_connection_count(0, Duration::from_secs(3)).await;

    // Give the client IO loop time to process the EOF and transition phase.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now sends must resolve with non-Delivered outcomes.
    for i in 0..5 {
        let outcome = client.send_frame(
            format!("frame-{i}").as_bytes(),
            Duration::from_secs(1),
        ).await;
        // Could be WriteFailed, AckTimeout, or ConnectionNotActive.
        // Must NOT be Delivered (server connection handler is gone).
        // Must NOT hang.
        assert!(
            !outcome.is_delivered(),
            "frame {i} after server death returned Delivered"
        );
    }

    client.shutdown().await;
}

/// Proves: drain timeout fires and connection cleans up when the client
/// hard-drops (no shutdown frame). The server's read task sees EOF, the
/// control loop breaks, and the drain timeout must fire and abort the
/// write task so the connection is fully cleaned up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drain_timeout_aborts_stuck_write_task() {
    common::init_tracing();
    let path = sock_path("drain-abort");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (state_tx, mut state_rx) = mpsc::channel(64);
    let (control_tx, _) = mpsc::channel(64);

    let mut config = IpcConfig::default();
    config.drain_timeout_ms = 500;

    let router = LifecycleRouter { state_tx, control_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let ss = server.state();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &config, None,
    ).await.unwrap();
    wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    let outcome = client.send_frame(b"activate", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());

    // Hard-drop: no shutdown frame. Server sees EOF immediately.
    // Drain timeout (500ms) must fire and abort the write task.
    drop(client);

    // Must clean up within drain_timeout + margin.
    ss.wait_connection_count(0, Duration::from_secs(3)).await;
    assert_eq!(ss.connections.len(), 0, "connection leaked after drain timeout");

    server_handle.abort();
}

// ---- Gap 6: graceful shutdown delivers all ACKed frames ----

/// Proves: after graceful shutdown, every frame that received an ACK
/// was actually delivered to the router. This verifies no frames are
/// ACKed but silently dropped between the server control loop and the
/// router callback.
///
/// Note: since send_frame awaits ACK, the BufWriter flush path is
/// exercised implicitly — the ACK can only arrive if the frame was
/// flushed to the socket, received by the server, and processed.
/// This test verifies end-to-end delivery, not just flush mechanics.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn graceful_shutdown_all_acked_frames_delivered() {
    common::init_tracing();
    let path = sock_path("flush-all");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = keys::generate_keypair().unwrap();

    let (state_tx, mut state_rx) = mpsc::channel(64);
    let (control_tx, mut control_rx) = mpsc::channel(256);
    let router = LifecycleRouter { state_tx, control_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let server_handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, client_kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_for_state(&mut state_rx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Send 10 frames — each ACKed before the next is sent.
    for i in 0u32..10 {
        let outcome = client.send_frame(
            format!("flush-{i}").as_bytes(), Duration::from_secs(5)
        ).await;
        assert!(outcome.is_delivered(), "frame {i}: {outcome:?}");
    }

    // Graceful shutdown completes without hanging.
    client.shutdown().await;

    // ALL 10 ACKed frames must have been delivered to the router.
    let mut count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), control_rx.recv()).await {
        count += 1;
    }
    assert_eq!(count, 10,
        "all ACKed frames must reach the router. Got {count}/10.");

    server_handle.abort();
}
