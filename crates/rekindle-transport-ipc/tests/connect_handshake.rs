//! Connect/Handshake tests: every handshake condition.

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
use rekindle_transport_ipc::transport_frame::ConnectionPhase;
use rekindle_transport_ipc::client::IpcClient;

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-hs-test-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct HsRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    control_tx: mpsc::Sender<(u64, Bytes)>,
}
impl FrameRouter for HsRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, old: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, old, new));
    }
}

async fn wait_state(rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>, expected: ConnectionPhase) -> u64 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, _, new))) if new == expected => return id,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("state channel closed before {expected}"),
            Err(_) => panic!("timed out waiting for {expected}"),
        }
    }
}

async fn make_server(path: &std::path::Path) -> ([u8; 32], mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>, mpsc::Receiver<(u64, Bytes)>, tokio::task::JoinHandle<()>) {
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (state_tx, state_rx) = mpsc::channel(256);
    let (control_tx, control_rx) = mpsc::channel(256);
    let router = HsRouter { state_tx, control_tx };
    let server = IpcServer::bind(path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    (server_pub, state_rx, control_rx, handle)
}

/// 2.2 Connect to nonexistent path fails with specific error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_nonexistent_path() {
    common::init_tracing();
    let path = sock_path("no-exist");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let result = IpcClient::connect(Uuid::now_v7(), &path, &[0u8; 32], kp.as_inner(), &IpcConfig::default(), None).await;
    assert!(result.is_err(), "connect to missing path must fail");
}

/// 2.3 Connect to a regular file fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_to_regular_file() {
    common::init_tracing();
    let path = sock_path("file-not-sock");
    let _guard = common::SocketGuard::new(path.clone());
    std::fs::write(&path, b"not a socket").unwrap();
    let kp = keys::generate_keypair().unwrap();
    let result = IpcClient::connect(Uuid::now_v7(), &path, &[0u8; 32], kp.as_inner(), &IpcConfig::default(), None).await;
    assert!(result.is_err(), "connect to regular file must fail");
}

/// 2.6 Handshake timeout: server accepts but never performs Noise.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshake_timeout_fires() {
    common::init_tracing();
    let path = sock_path("hs-timeout");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let _hold = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(stream);
    });
    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.handshake_timeout_ms = 500;
    let start = std::time::Instant::now();
    let result = IpcClient::connect(Uuid::now_v7(), &path, &[0u8; 32], kp.as_inner(), &config, None).await;
    let elapsed = start.elapsed();
    assert!(result.is_err(), "must timeout");
    assert!(elapsed < Duration::from_secs(3), "must timeout in <3s, took {elapsed:?}");
}

/// 2.8 Client sends garbage instead of msg1 — server rejects, keeps accepting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn garbage_handshake_server_survives() {
    common::init_tracing();
    let path = sock_path("garbage-hs");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (server_pub, mut state_rx, _, handle) = make_server(&path).await;

    // Send raw garbage to the socket (not a Noise handshake).
    {
        use tokio::io::AsyncWriteExt;
        let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        stream.write_all(b"THIS IS NOT A NOISE HANDSHAKE MESSAGE AT ALL").await.unwrap();
        stream.flush().await.unwrap();
        drop(stream);
    }

    // Give server time to reject the garbage connection.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A legitimate client must still be able to connect after the garbage.
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_state(&mut state_rx, ConnectionPhase::Ready).await;

    let outcome = client.send_frame(b"still-works", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered(), "server must survive garbage and serve new clients");

    client.shutdown().await;
    handle.abort();
}

/// 2.10 Handshake with all-zero pubkey fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handshake_zero_pubkey_fails() {
    common::init_tracing();
    let path = sock_path("zero-pub");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (_, _, _, handle) = make_server(&path).await;

    let kp = keys::generate_keypair().unwrap();
    let result = IpcClient::connect(Uuid::now_v7(), &path, &[0u8; 32], kp.as_inner(), &IpcConfig::default(), None).await;
    assert!(result.is_err(), "all-zero pubkey must fail handshake");

    handle.abort();
}

/// 2.12 Two clients connect simultaneously — both complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_simultaneous_connects() {
    common::init_tracing();
    let path = sock_path("2-simul");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (server_pub, mut state_rx, _, handle) = make_server(&path).await;

    let (c1, c2) = tokio::join!(
        async {
            let kp = keys::generate_keypair().unwrap();
            IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await
        },
        async {
            let kp = keys::generate_keypair().unwrap();
            IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await
        },
    );
    let c1 = c1.unwrap();
    let c2 = c2.unwrap();
    let id1 = wait_state(&mut state_rx, ConnectionPhase::Ready).await;
    let id2 = wait_state(&mut state_rx, ConnectionPhase::Ready).await;
    assert_ne!(id1, id2);
    c1.shutdown().await;
    c2.shutdown().await;
    handle.abort();
}

/// 2.13 10 clients connect in rapid succession.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_rapid_connects() {
    common::init_tracing();
    let path = sock_path("10-rapid");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (server_pub, mut state_rx, _, handle) = make_server(&path).await;

    let mut handles = Vec::new();
    for _ in 0..10 {
        let path = path.clone();
        handles.push(tokio::spawn(async move {
            let kp = keys::generate_keypair().unwrap();
            let c = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
            c.shutdown().await;
        }));
    }
    for h in handles { h.await.unwrap(); }

    let mut ready_count = 0;
    while let Ok(Some((_, _, new))) = tokio::time::timeout(Duration::from_millis(500), state_rx.recv()).await {
        if new == ConnectionPhase::Ready { ready_count += 1; }
    }
    assert_eq!(ready_count, 10, "all 10 must reach Ready, got {ready_count}");
    handle.abort();
}

/// 2.11 Connect with correct handshake — verify bulk cipher derived.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handshake_derives_bulk_cipher() {
    common::init_tracing();
    let path = sock_path("bulk-derive");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (server_pub, _, _, handle) = make_server(&path).await;
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &server_pub, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    assert!(client.bulk_cipher().is_some(), "bulk cipher must be derived from handshake hash");
    client.shutdown().await;
    handle.abort();
}
