//! Recovery after transient failure.
//!
//! Proves: client sends successfully, server slows down causing ack timeouts,
//! server speeds back up, client sends successfully again. The transport
//! must recover from transient ack timeouts without permanent degradation.
//!
//! WILL FAIL if the transport enters a permanently broken state after
//! experiencing ack timeouts — e.g., if the IO loop breaks on timeout
//! instead of continuing, or if pending_acks leaks entries that block
//! future sends.

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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
        "rekindle-recovery-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

/// Router with configurable per-frame delay via shared atomic.
/// delay_ms=0 means no delay. Set to >ack_timeout to cause timeouts.
struct DelayRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    delay_ms: Arc<AtomicU64>,
}
impl FrameRouter for DelayRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) {
        let delay = self.delay_ms.load(Ordering::Relaxed);
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        let _ = self.control_tx.try_send((id, p));
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
}

/// Phase 1: send 10 frames with no delay — all Delivered.
/// Phase 2: set delay to 6000ms (exceeds 5s ack timeout) — send 3 frames, all timeout.
/// Phase 3: set delay back to 0 — send 10 frames, all Delivered.
///
/// Proves the transport recovers from transient ack timeouts.
///
/// Uses a long heartbeat interval (60s) to prevent heartbeat dead-peer
/// detection from firing during Phase 2. The `route_frame` delay blocks
/// the server's control loop for 3×6s = 18s. Default heartbeat (14s
/// detection) would declare the connection dead before Phase 2 completes.
/// The test isolates ack-timeout recovery from heartbeat detection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovery_after_ack_timeout() {
    common::init_tracing();
    let path = sock_path("recovery");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    // Heartbeat must not fire during Phase 2's 18s blocking period.
    // Set interval to 60s so no pings are sent during the test.
    let mut config = IpcConfig::default();
    config.heartbeat_interval_ms = 60_000;
    config.idle_timeout_ms = 0; // disable idle timeout

    let delay_ms = Arc::new(AtomicU64::new(0));
    let (control_tx, mut control_rx) = mpsc::channel(256);
    let router = DelayRouter {
        control_tx,
        delay_ms: Arc::clone(&delay_ms),
    };

    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &config, None,
    ).await.unwrap();

    // Phase 1: no delay, all succeed.
    for i in 0u32..10 {
        let outcome = client.send_frame(format!("phase1-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "phase1 frame {i}: {outcome:?}");
    }

    // Drain delivered frames.
    let mut phase1_count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), control_rx.recv()).await {
        phase1_count += 1;
    }
    assert_eq!(phase1_count, 10, "phase1: expected 10, got {phase1_count}");

    // Phase 2: set delay to 6s (exceeds 5s ack timeout). Sends should timeout.
    delay_ms.store(6000, Ordering::Relaxed);

    for i in 0u32..3 {
        let outcome = client.send_frame(format!("phase2-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(
            !outcome.is_delivered(),
            "phase2 frame {i} must timeout, got: {outcome:?}"
        );
    }

    // Phase 3: remove delay. Sends must succeed again.
    delay_ms.store(0, Ordering::Relaxed);

    // Drain any phase2 frames that eventually complete server-side.
    // The server's router callback was blocked for 6s per frame.
    // Those frames will eventually be processed and their acks sent,
    // but the client already timed out on them. We need to let the
    // server finish processing them before sending phase3 frames,
    // otherwise the server's router is still blocked on a phase2 frame.
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(20), control_rx.recv()).await {
        // drain until no more phase2 frames
    }

    for i in 0u32..10 {
        let outcome = client.send_frame(format!("phase3-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(
            outcome.is_delivered(),
            "phase3 frame {i} must succeed after recovery: {outcome:?}. \
             The transport must recover from transient ack timeouts."
        );
    }

    client.shutdown().await;
    handle.abort();
}
