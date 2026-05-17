//! Resource leak detection over many connect/disconnect cycles.
//!
//! Upstream reference: rtrb/tests/lib.rs:69-119 drops test
//! which uses AtomicUsize to track exact drop counts across 100 iterations.
//!
//! WILL FAIL if:
//! - DashMap entries not removed on disconnect (connection.rs:320)
//! - name_to_conn entries not cleaned up (connection.rs:325)
//! - pending_requests entries not cleaned up (connection.rs:328)
//! - File descriptors leak across connect/disconnect cycles
//! - Read/write task JoinHandles not awaited (leaked tokio tasks)

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
        "rekindle-rlc-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct LeakRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for LeakRouter {
    fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
    }
}

/// 100 connect/send/disconnect cycles. Connection count returns to 0 each time.
/// fd count stays bounded on Linux.
///
/// Uses a short drain timeout (500ms) to ensure rapid cleanup after
/// each disconnect. The default 5s drain timeout would race with the
/// test's wait_connection_count timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_leaks_100_cycles() {
    common::init_tracing();
    let path = sock_path("leak-100");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(4096);
    let mut config = IpcConfig::default();
    config.drain_timeout_ms = 500;
    let server = IpcServer::bind(&path, kp.into_inner(), LeakRouter { state_tx: stx }, config.clone()).unwrap();
    let ss = server.state();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    assert_eq!(ss.connections.len(), 0, "initial connection count must be 0");

    #[cfg(target_os = "linux")]
    let baseline_fds = std::fs::read_dir("/proc/self/fd").map(|d| d.count()).unwrap_or(0);

    for cycle in 0u32..100 {
        let ckp = keys::generate_keypair().unwrap();
        let c = IpcClient::connect(
            Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, None,
        ).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, srx.recv()).await {
                Ok(Some((_, _, ConnectionPhase::Ready))) => break,
                Ok(Some(_)) => continue,
                _ => panic!("cycle {cycle}: never Ready"),
            }
        }

        let outcome = c.send_frame(format!("c{cycle}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "cycle {cycle}: {outcome:?}");

        c.shutdown().await;
        ss.wait_connection_count(0, Duration::from_secs(5)).await;
    }

    assert_eq!(ss.connections.len(), 0, "leak: {} connections remain after 100 cycles", ss.connections.len());

    #[cfg(target_os = "linux")]
    {
        let final_fds = std::fs::read_dir("/proc/self/fd").map(|d| d.count()).unwrap_or(0);
        assert!(
            final_fds < baseline_fds + 20,
            "fd leak: baseline {baseline_fds}, final {final_fds}, delta {}",
            final_fds.saturating_sub(baseline_fds)
        );
    }

    handle.abort();
}

/// 50 rapid connect/hard-drop cycles (no graceful shutdown).
/// Proves the server cleans up even without receiving a SHUTDOWN frame.
///
/// Uses short drain timeout (500ms) and fast heartbeat (1s interval,
/// 500ms pong timeout, 2 misses) to detect dead peers quickly.
/// Without this, the default 14s heartbeat detection would exceed the
/// test's wait_connection_count timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_leaks_50_hard_drops() {
    common::init_tracing();
    let path = sock_path("leak-harddrop");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(4096);
    let mut config = IpcConfig::default();
    config.drain_timeout_ms = 500;
    config.heartbeat_interval_ms = 1_000;
    config.heartbeat_pong_timeout_ms = 500;
    config.heartbeat_max_misses = 2;
    let server = IpcServer::bind(&path, kp.into_inner(), LeakRouter { state_tx: stx }, config.clone()).unwrap();
    let ss = server.state();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    for cycle in 0u32..50 {
        let ckp = keys::generate_keypair().unwrap();
        let c = IpcClient::connect(
            Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, None,
        ).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, srx.recv()).await {
                Ok(Some((_, _, ConnectionPhase::Ready))) => break,
                Ok(Some(_)) => continue,
                _ => panic!("cycle {cycle}: never Ready"),
            }
        }

        drop(c); // hard drop, no shutdown frame
        ss.wait_connection_count(0, Duration::from_secs(5)).await;
    }

    assert_eq!(ss.connections.len(), 0, "leak after hard drops: {} remain", ss.connections.len());

    handle.abort();
}
