use std::sync::Arc;
use std::time::Duration;

use rekindle_transport_ipc::v3::context::ServerConfig;
use rekindle_transport_ipc::v3::crypto::noise::{build_prologue, generate_keypair};
use rekindle_transport_ipc::v3::router::{CapturedStateChange, ConnectionPhase, MockRouter, ReplyRouter};
use rekindle_transport_ipc::v3::server::{ConnectionHandle, IpcServer};
use rekindle_transport_ipc::v3::session::handshake::{dialler_handshake, HandshakeConfig};
use rekindle_transport_ipc::v3::socket::{extract_ucred, PeerCredentials};
use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

use super::harness::*;

async fn wait_for_phase(router: &MockRouter, expected: ConnectionPhase, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let changes = router.state_changes.lock();
        if changes.iter().any(|sc| sc.new_phase == expected) {
            tracing::info!(?expected, count = changes.len(), "wait_for_phase: found");
            return true;
        }
        let count = changes.len();
        drop(changes);
        if tokio::time::Instant::now() >= deadline {
            tracing::error!(?expected, count, "wait_for_phase: TIMEOUT");
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn state_changes(router: &MockRouter) -> Vec<CapturedStateChange> {
    router.state_changes.lock().clone()
}

/// Heartbeat miss must produce Degraded before Dead.
///
/// Connects a raw socket, completes Noise handshake, then holds the
/// socket open without reading. The server sends PINGs into the socket
/// buffer. Nobody reads them. Nobody sends PONGs. Heartbeat fires:
/// Established → Degraded → Dead. The socket stays open the entire
/// time — no EOF races the heartbeat.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_miss_produces_degraded_before_dead() {
    init_tracing();
    tracing::info!("test: starting heartbeat_miss_produces_degraded_before_dead");

    let tempdir = tempfile::tempdir().unwrap();
    let socket_path = tempdir.path().join("heartbeat_degraded.sock");
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public.clone().try_into().unwrap();

    let router = MockRouter::new();
    let router_ref = Arc::clone(&router);

    let mut config = ServerConfig::for_test();
    config.session.heartbeat_interval_ms = 200;
    config.session.heartbeat_response_timeout_ms = 300;
    config.session.heartbeat_miss_limit = 3;
    tracing::info!(
        heartbeat_interval_ms = config.session.heartbeat_interval_ms,
        heartbeat_response_timeout_ms = config.session.heartbeat_response_timeout_ms,
        heartbeat_miss_limit = config.session.heartbeat_miss_limit,
        "test: server config",
    );

    let server = IpcServer::bind(
        &socket_path,
        server_kp,
        move |handle: ConnectionHandle| {
            ReplyRouter {
                router: Arc::clone(&router_ref),
                conn_handle: handle,
            }
        },
        config,
    ).await.unwrap();
    tracing::info!("test: server bound");

    let cancel = server.cancel_token().clone();
    tokio::spawn(async move { let _ = server.run().await; });
    tokio::time::sleep(Duration::from_millis(50)).await;
    tracing::info!("test: server spawned, connecting raw socket");

    let mut stream = tokio::net::UnixStream::connect(&socket_path).await
        .expect("raw socket connect must succeed");
    tracing::info!("test: raw socket connected");

    let peer_creds = extract_ucred(&stream)
        .expect("UCred extraction must succeed");
    let local_creds = PeerCredentials::local();
    let prologue = build_prologue(
        local_creds.pid, local_creds.uid,
        peer_creds.pid, peer_creds.uid,
    ).expect("prologue must succeed");

    let hs_config = HandshakeConfig::new(
        CapabilityBits::MANDATORY_V1 | CapabilityBits::AEAD_AEGIS128L,
        Clearance::Internal,
    );

    let _handshake_result = dialler_handshake(
        &mut stream,
        &client_kp,
        &server_pub,
        hs_config,
        &prologue,
        Duration::from_secs(5),
    ).await.expect("handshake must succeed");
    tracing::info!("test: handshake complete — going silent, holding socket open");

    // DO NOT read from the stream. DO NOT close it.
    // Server sends PINGs. Nobody responds. Heartbeat fires.

    tracing::info!("test: waiting for ConnectionPhase::Dead (10s timeout)");
    let saw_dead = wait_for_phase(&router, ConnectionPhase::Dead, Duration::from_secs(10)).await;

    let changes = state_changes(&router);
    let saw_degraded = changes.iter().any(|sc| sc.new_phase == ConnectionPhase::Degraded);
    tracing::info!(saw_dead, saw_degraded, transition_count = changes.len(), "test: wait complete");

    drop(stream);
    tracing::info!("test: raw socket dropped");

    assert!(
        saw_dead,
        "ConnectionPhase::Dead never fired within 10s — \
         heartbeat did not detect unresponsive peer. Transitions: {changes:?}",
    );
    assert!(
        saw_degraded,
        "ConnectionPhase::Degraded must fire before Dead — \
         transport went directly to Dead without surfacing degraded. Transitions: {changes:?}",
    );

    let degraded_idx = changes.iter().position(|sc| sc.new_phase == ConnectionPhase::Degraded);
    let dead_idx = changes.iter().position(|sc| sc.new_phase == ConnectionPhase::Dead);
    if let (Some(d), Some(dd)) = (degraded_idx, dead_idx) {
        assert!(
            d < dd,
            "Degraded (index {d}) must appear before Dead (index {dd}) — transitions: {changes:?}",
        );
    }

    cancel.cancel();
    tracing::info!("test: PASSED");
}

/// Graceful client shutdown must produce ConnectionPhase::Closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_produces_closed() {
    init_tracing();
    tracing::info!("test: starting graceful_shutdown_produces_closed");

    let mut f = connected_pair().await;
    tracing::info!("test: connected pair ready");

    f.send_request(b"hello", TEST_TIMEOUT).await.unwrap();
    tracing::info!("test: request sent, shutting down client");
    f.take_client().shutdown().await;
    tracing::info!("test: client shutdown complete, waiting for Closed");

    assert!(
        wait_for_phase(&f.router, ConnectionPhase::Closed, Duration::from_secs(5)).await,
        "ConnectionPhase::Closed never fired within 5s — transitions: {:?}",
        state_changes(&f.router),
    );
    tracing::info!("test: PASSED");
}
