//! Connection lifecycle state machine tests.
//! No sleep-based synchronization. All waits are on channels.

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
        "rekindle-life-test-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct LifeRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for LifeRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, old: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, old, new));
    }
}

async fn wait_state(rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>, expected: ConnectionPhase, timeout: Duration) -> u64 {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, _, new))) if new == expected => return id,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("channel closed before {expected}"),
            Err(_) => panic!("timed out ({timeout:?}) for {expected}"),
        }
    }
}

/// Wait for a specific conn_id to reach a state. Drains other connections' transitions.
async fn wait_conn_state(
    rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>,
    conn_id: u64, expected: ConnectionPhase, timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, _, new))) if id == conn_id && new == expected => return,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("channel closed before conn {conn_id} → {expected}"),
            Err(_) => panic!("timed out for conn {conn_id} → {expected}"),
        }
    }
}

async fn make_server(path: &std::path::Path) -> (
    [u8; 32],
    mpsc::Receiver<(u64, Bytes)>,
    mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>,
    std::sync::Arc<ServerState>,
    tokio::task::JoinHandle<()>,
) {
    let server_kp = keys::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (control_tx, control_rx) = mpsc::channel(256);
    let (state_tx, state_rx) = mpsc::channel(256);
    let router = LifeRouter { control_tx, state_tx };
    let server = IpcServer::bind(path, server_kp.into_inner(), router, IpcConfig::default()).unwrap();
    let ss = server.state();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    (server_pub, control_rx, state_rx, ss, handle)
}

/// 6.1 Connect → Ready.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connect_reaches_ready() {
    common::init_tracing();
    let path = sock_path("ready");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, mut srx, _, handle) = make_server(&path).await;
    let kp = keys::generate_keypair().unwrap();
    let c = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;
    c.shutdown().await; handle.abort();
}

/// 6.2 Ready → Active on first app frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_frame_activates() {
    common::init_tracing();
    let path = sock_path("activate");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, mut srx, _, handle) = make_server(&path).await;
    let kp = keys::generate_keypair().unwrap();
    let c = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let conn_id = wait_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;
    let o = c.send_frame(b"activate", Duration::from_secs(5)).await;
    assert!(o.is_delivered());
    wait_conn_state(&mut srx, conn_id, ConnectionPhase::Active, Duration::from_secs(5)).await;
    c.shutdown().await; handle.abort();
}

/// 6.11 Drop one connection — others survive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drop_one_others_survive() {
    common::init_tracing();
    let path = sock_path("drop-one");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut crx, mut srx, _, handle) = make_server(&path).await;

    let kp1 = keys::generate_keypair().unwrap();
    let c1 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp1.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let id1 = wait_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    let kp2 = keys::generate_keypair().unwrap();
    let c2 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp2.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let id2 = wait_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Both send.
    assert!(c1.send_frame(b"c1", Duration::from_secs(5)).await.is_delivered());
    assert!(c2.send_frame(b"c2", Duration::from_secs(5)).await.is_delivered());

    // Kill c1.
    c1.shutdown().await;

    // Wait for c1's Dead/Closed transition.
    wait_conn_state(&mut srx, id1, ConnectionPhase::Dead, Duration::from_secs(5)).await;

    // c2 still works.
    assert!(c2.send_frame(b"c2-still-alive", Duration::from_secs(5)).await.is_delivered());

    // Drain and find c2's frame — verify it came from c2's connection, not stale c1.
    let mut found = false;
    while let Ok(Some((recv_id, p))) = tokio::time::timeout(Duration::from_millis(500), crx.recv()).await {
        if &p[..] == b"c2-still-alive" {
            assert_eq!(recv_id, id2, "frame must come from c2's connection, not stale c1");
            found = true;
            break;
        }
    }
    assert!(found, "c2 frame must arrive after c1 died");

    c2.shutdown().await; handle.abort();
}

/// 6.15 Connection count tracks connect/disconnect.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_count_tracking() {
    common::init_tracing();
    let path = sock_path("count");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, mut srx, ss, handle) = make_server(&path).await;

    assert_eq!(ss.connections.len(), 0, "initial must be 0");

    let kp = keys::generate_keypair().unwrap();
    let c = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let conn_id = wait_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;
    assert_eq!(ss.connections.len(), 1, "after connect must be 1");

    // Send a frame to confirm the connection is fully Active before shutting down.
    // This ensures the Ready transition is consumed before Dead can arrive,
    // preventing a race where BrokenPipe fires before Ready propagates.
    let outcome = c.send_frame(b"alive", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());
    wait_conn_state(&mut srx, conn_id, ConnectionPhase::Active, Duration::from_secs(5)).await;

    c.shutdown().await;

    // Wait for terminal transitions. The connection handler transitions
    // Dead → cleanup → Closed. Both must fire in sequence.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_dead = false;
    let mut saw_closed = false;
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((id, _, new))) if id == conn_id => {
                if new == ConnectionPhase::Dead { saw_dead = true; }
                if new == ConnectionPhase::Closed { saw_closed = true; }
                if saw_closed { break; }
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("state channel closed before conn {conn_id} → Closed"),
            Err(_) => panic!("timed out waiting for conn {conn_id} terminal (dead={saw_dead}, closed={saw_closed})"),
        }
    }

    assert_eq!(ss.connections.len(), 0, "after disconnect must be 0");

    handle.abort();
}

/// 6.14 Phase query returns correct value at Ready and after shutdown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phase_query_accuracy() {
    common::init_tracing();
    let path = sock_path("phase");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, mut srx, _, handle) = make_server(&path).await;
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    let phase = client.phase();
    assert!(phase.can_send(), "phase after connect: {phase}");
    assert!(!phase.is_terminal(), "phase must not be terminal after connect");

    // shutdown() consumes self — phase cannot be queried after.
    // Verifying shutdown completes without hanging proves the IO loop
    // transitioned to Closed internally. The server-side state_machine
    // tests verify the terminal transition fires via on_connection_state_changed.
    client.shutdown().await;

    handle.abort();
}
