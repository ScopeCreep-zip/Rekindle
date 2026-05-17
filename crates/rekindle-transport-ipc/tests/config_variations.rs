//! Configuration variation tests: non-default configs that must work.

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
        "rekindle-cfg-test-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct CfgRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for CfgRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, _: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, ConnectionPhase::Handshaking, new));
    }
}

async fn wait_ready(rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => return,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready"),
        }
    }
}

/// 9.2 Custom buffer sizes — transport works.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn custom_buffer_sizes() {
    common::init_tracing();
    let path = sock_path("custom-buf");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (control_tx, mut control_rx) = mpsc::channel(64);
    let (state_tx, mut state_rx) = mpsc::channel(64);

    let mut config = IpcConfig::default();
    config.uds_sndbuf = Some(1_048_576); // 1 MiB
    config.uds_rcvbuf = Some(1_048_576);
    config.pool_slab_count = 32; // smaller pool

    let router = CfgRouter { control_tx, state_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let kp = keys::generate_keypair().unwrap();
    let c = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &config, None).await.unwrap();
    wait_ready(&mut state_rx).await;

    let outcome = c.send_frame(b"custom-buf-works", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());
    let (_, p) = tokio::time::timeout(Duration::from_secs(5), control_rx.recv()).await.unwrap().unwrap();
    assert_eq!(&p[..], b"custom-buf-works");

    c.shutdown().await;
    handle.abort();
}

/// 9.3 Minimum pool slab count (1) — still works for small payloads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn minimum_pool_slabs() {
    common::init_tracing();
    let path = sock_path("min-pool");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (control_tx, mut control_rx) = mpsc::channel(64);
    let (state_tx, mut state_rx) = mpsc::channel(64);

    let mut config = IpcConfig::default();
    config.pool_slab_count = 1;

    let router = CfgRouter { control_tx, state_tx };
    let server = IpcServer::bind(&path, server_kp.into_inner(), router, config.clone()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let kp = keys::generate_keypair().unwrap();
    let c = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &config, None).await.unwrap();
    wait_ready(&mut state_rx).await;

    let outcome = c.send_frame(b"min-pool", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());
    let (_, p) = tokio::time::timeout(Duration::from_secs(5), control_rx.recv()).await.unwrap().unwrap();
    assert_eq!(&p[..], b"min-pool");

    c.shutdown().await;
    handle.abort();
}

/// 9.4 Custom handshake timeout fires at configured value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_handshake_timeout() {
    common::init_tracing();
    let path = sock_path("custom-hs-to");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    // Raw listener that never handshakes.
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let _hold = tokio::spawn(async move {
        let (s, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
        drop(s);
    });

    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.handshake_timeout_ms = 300; // 300ms

    let start = std::time::Instant::now();
    let result = IpcClient::connect(Uuid::now_v7(), &path, &[0u8; 32], kp.as_inner(), &config, None).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "must timeout");
    assert!(elapsed >= Duration::from_millis(250), "must wait at least ~300ms, waited {elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "must not wait >2s, waited {elapsed:?}");
}

/// 9.5 Every config field validated — comprehensive.
#[test]
fn config_validate_comprehensive() {
    common::init_tracing();
    // Valid default.
    assert!(IpcConfig::default().validate().is_ok());

    // Each invalid field.
    let cases: Vec<(&str, Box<dyn Fn(&mut IpcConfig)>)> = vec![
        ("max_frame_size=0", Box::new(|c: &mut IpcConfig| c.max_frame_size = 0)),
        ("max_frame_size=3", Box::new(|c| c.max_frame_size = 3)),
        ("max_frame_size=128MiB", Box::new(|c| c.max_frame_size = 128 * 1024 * 1024)),
        ("max_connections=0", Box::new(|c| c.max_connections = 0)),
        ("listen_backlog=0", Box::new(|c| c.listen_backlog = 0)),
        ("pool_slab_count=0", Box::new(|c| c.pool_slab_count = 0)),
        ("handshake_timeout_ms=0", Box::new(|c| c.handshake_timeout_ms = 0)),
    ];

    for (name, mutate) in cases {
        let mut config = IpcConfig::default();
        mutate(&mut config);
        assert!(config.validate().is_err(), "config with {name} must fail validation");
    }

    // Valid non-default configs.
    let valid_cases: Vec<(&str, Box<dyn Fn(&mut IpcConfig)>)> = vec![
        ("max_frame_size=4", Box::new(|c: &mut IpcConfig| c.max_frame_size = 4)),
        ("max_frame_size=64MiB", Box::new(|c| c.max_frame_size = 64 * 1024 * 1024)),
        ("max_connections=1", Box::new(|c| c.max_connections = 1)),
        ("pool_slab_count=1", Box::new(|c| c.pool_slab_count = 1)),
        ("handshake_timeout_ms=1", Box::new(|c| c.handshake_timeout_ms = 1)),
    ];

    for (name, mutate) in valid_cases {
        let mut config = IpcConfig::default();
        mutate(&mut config);
        assert!(config.validate().is_ok(), "config with {name} must pass validation");
    }
}
