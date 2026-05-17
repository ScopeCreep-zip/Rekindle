//! max_frame_size enforcement tests.
//!
//! WILL FAIL if:
//! - send_frame doesn't check config.max_frame_size (only checks hardcoded MAX_FRAME_SIZE)
//! - Server doesn't reject oversized frames
//! - Connection dies after size rejection instead of staying alive
//!
//! The NoiseWriter at writer.rs:37-42 checks against the hardcoded
//! MAX_FRAME_SIZE const (16MiB). The config's max_frame_size field
//! (config.rs:30) exists but may not be consulted in the send path.
//! This test forces that check to be added.

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
        "rekindle-mfe-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct MfeRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for MfeRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
    }
}

/// Frame at exactly max_frame_size: MUST succeed.
/// Frame at max_frame_size + 1: MUST fail without panicking or hanging.
/// Connection MUST survive the rejection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn max_frame_boundary() {
    common::init_tracing();
    let path = sock_path("mfe-boundary");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();

    let mut config = IpcConfig::default();
    config.max_frame_size = 10_000;

    let (ctx, mut crx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = MfeRouter { control_tx: ctx, state_tx: stx };
    let server = IpcServer::bind(&path, kp.into_inner(), router, config.clone()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, None).await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready"),
        }
    }

    // Exact max: MUST succeed.
    let exact = vec![0xAA; 10_000];
    let outcome = client.send_frame(&exact, Duration::from_secs(10)).await;
    assert!(outcome.is_delivered(), "exact max_frame_size: {outcome:?}");
    let (_, p) = tokio::time::timeout(Duration::from_secs(5), crx.recv()).await.unwrap().unwrap();
    assert_eq!(p.len(), 10_000);

    // One over max: MUST fail.
    let over = vec![0xBB; 10_001];
    let outcome = client.send_frame(&over, Duration::from_secs(5)).await;
    assert!(
        !outcome.is_delivered(),
        "frame exceeding max_frame_size must not be delivered: {outcome:?}"
    );

    // Connection MUST still be alive after the rejection.
    let small = vec![0xCC; 100];
    let outcome = client.send_frame(&small, Duration::from_secs(5)).await;
    assert!(outcome.is_delivered(), "connection must survive size rejection: {outcome:?}");

    client.shutdown().await;
    handle.abort();
}

/// Multiple oversized rejections in a row — connection must survive all of them.
/// Proves rejection is non-destructive and doesn't leak state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repeated_oversized_rejections_connection_survives() {
    common::init_tracing();
    let path = sock_path("mfe-repeated");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();

    let mut config = IpcConfig::default();
    config.max_frame_size = 5_000;

    let (ctx, mut crx) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(64);
    let router = MfeRouter { control_tx: ctx, state_tx: stx };
    let server = IpcServer::bind(&path, kp.into_inner(), router, config.clone()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, None).await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready"),
        }
    }

    // Send 5 oversized frames in a row.
    for i in 0..5u32 {
        let oversized = vec![0xEE; 5_001];
        let outcome = client.send_frame(&oversized, Duration::from_secs(3)).await;
        assert!(
            !outcome.is_delivered(),
            "oversized frame {i} must not be delivered: {outcome:?}"
        );
    }

    // Connection must still work after 5 rejections.
    let small = vec![0xFF; 100];
    let outcome = client.send_frame(&small, Duration::from_secs(5)).await;
    assert!(
        outcome.is_delivered(),
        "connection must survive 5 consecutive oversized rejections: {outcome:?}"
    );

    let (_, p) = tokio::time::timeout(Duration::from_secs(5), crx.recv()).await.unwrap().unwrap();
    assert_eq!(p.len(), 100);

    client.shutdown().await;
    handle.abort();
}
