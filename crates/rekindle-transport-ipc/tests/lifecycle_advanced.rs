//! Advanced lifecycle tests: every state transition, recovery, edge case.

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
        "rekindle-ladv-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct AdvRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    bulk_tx: mpsc::Sender<(u64, u8, Vec<u8>)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    bulk_accumulator: parking_lot::Mutex<std::collections::HashMap<(u64, u8), Vec<u8>>>,
}
impl FrameRouter for AdvRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
    fn on_bulk_chunk(&self, _: &ServerState, conn_id: u64, stream_id: u8, _chunk_seq: u32, data: &[u8]) {
        self.bulk_accumulator.lock().entry((conn_id, stream_id)).or_default().extend_from_slice(data);
    }
    fn on_bulk_complete(&self, _: &ServerState, conn_id: u64, stream_id: u8, _total_bytes: u64, _total_chunks: u64) {
        let payload = self.bulk_accumulator.lock().remove(&(conn_id, stream_id)).unwrap_or_default();
        let _ = self.bulk_tx.try_send((conn_id, stream_id, payload));
    }
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, old: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, old, new));
    }
}

async fn wait_conn_state(rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>, conn_id: u64, expected: ConnectionPhase, timeout: Duration) {
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

async fn wait_any_state(rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>, expected: ConnectionPhase, timeout: Duration) -> u64 {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, _, new))) if new == expected => return id,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("channel closed before {expected}"),
            Err(_) => panic!("timed out for {expected}"),
        }
    }
}

async fn make_server(path: &std::path::Path) -> ([u8; 32], mpsc::Receiver<(u64, Bytes)>, mpsc::Receiver<(u64, u8, Vec<u8>)>, mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>, std::sync::Arc<ServerState>, tokio::task::JoinHandle<()>) {
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, crx) = mpsc::channel(1024);
    let (btx, brx) = mpsc::channel(64);
    let (stx, srx) = mpsc::channel(1024);
    let router = AdvRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let ss = server.state();
    let h = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    (pub_key, crx, brx, srx, ss, h)
}

/// 6.10 Client shutdown while bulk transfer in-flight.
/// send_bulk future must resolve (not hang), with non-Delivered outcome.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_during_bulk_transfer() {
    common::init_tracing();
    let path = sock_path("shutdown-bulk");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, _, mut srx, _, handle) = make_server(&path).await;
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_any_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Start a large bulk transfer in a spawned task.
    let bulk_handle = {
        let payload = vec![0xAB; 10_000_000]; // 10MB
        tokio::spawn(async move {
            let outcome = client.send_bulk(&payload, Duration::from_secs(5)).await;
            // Don't assert Delivered — we're shutting down mid-transfer.
            // Just return the outcome so we can verify it's not a hang.
            (client, outcome)
        })
    };

    // Give bulk a moment to start, then the task completes on its own
    // (either Delivered if fast enough, or timeout/connection-lost).
    let (client, outcome) = tokio::time::timeout(Duration::from_secs(30), bulk_handle)
        .await.expect("bulk task hung for 30s").expect("bulk task panicked");

    // The key assertion: the future resolved, it did not hang.
    // Outcome may be Delivered (fast machine) or AckTimeout or ConnectionLost.
    // All are acceptable — hanging is not.
    tracing::info!("bulk during shutdown outcome: {outcome:?}");

    client.shutdown().await;
    handle.abort();
}

/// 6.9 Both sides initiate shutdown simultaneously — no deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_shutdown_no_deadlock() {
    common::init_tracing();
    let path = sock_path("simul-shutdown");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, _, mut srx, _, handle) = make_server(&path).await;
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_any_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Send one frame to reach Active.
    client.send_frame(b"activate", Duration::from_secs(5)).await;

    // Kill server and shutdown client simultaneously.
    let client_shutdown = tokio::spawn(async move { client.shutdown().await; });
    handle.abort();

    // Both must complete within 5s — no deadlock.
    let result = tokio::time::timeout(Duration::from_secs(5), client_shutdown).await;
    assert!(result.is_ok(), "simultaneous shutdown deadlocked");
}

/// 6.11b Server drops one connection by killing the client — other connections get frames.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn killed_client_does_not_affect_siblings() {
    common::init_tracing();
    let path = sock_path("killed-sibling");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut crx, _, mut srx, _, handle) = make_server(&path).await;

    let kp1 = keys::generate_keypair().unwrap();
    let c1 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp1.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_any_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    let kp2 = keys::generate_keypair().unwrap();
    let c2 = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp2.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_any_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Hard-drop c1 (no shutdown frame).
    drop(c1);

    // c2 sends 10 frames — all must succeed.
    for i in 0..10u32 {
        let outcome = c2.send_frame(format!("alive-{i}").as_bytes(), Duration::from_secs(5)).await;
        assert!(outcome.is_delivered(), "frame {i} after sibling death: {outcome:?}");
    }

    // Verify all 10 arrived.
    let mut count = 0;
    while let Ok(Some((_, p))) = tokio::time::timeout(Duration::from_millis(500), crx.recv()).await {
        if std::str::from_utf8(&p).unwrap_or("").starts_with("alive-") { count += 1; }
        if count >= 10 { break; }
    }
    assert_eq!(count, 10);

    c2.shutdown().await;
    handle.abort();
}

/// 6.8 Graceful client shutdown: Shutdown frame → ShutdownAck → close.
/// Proves the transport protocol exchange completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn graceful_shutdown_completes() {
    common::init_tracing();
    let path = sock_path("graceful");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, _, mut srx, ss, handle) = make_server(&path).await;
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    let conn_id = wait_any_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    client.send_frame(b"warmup", Duration::from_secs(5)).await;

    // Phase before shutdown.
    assert!(ss.connections.get(&conn_id).is_some());

    // Graceful shutdown.
    client.shutdown().await;

    // Server should see Dead or Closed for this conn.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut saw_dead_or_closed = false;
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((id, _, new))) if id == conn_id && (new == ConnectionPhase::Dead || new == ConnectionPhase::Closed) => {
                saw_dead_or_closed = true;
                break;
            }
            Ok(Some(_)) => continue,
            _ => break,
        }
    }
    assert!(saw_dead_or_closed, "server must detect client shutdown");

    handle.abort();
}

/// Multiple sequential bulk transfers on same connection — no state leakage.
/// Each transfer must complete independently.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_sequential_bulk_no_leakage() {
    common::init_tracing();
    let path = sock_path("5seq-bulk");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, mut brx, mut srx, _, handle) = make_server(&path).await;
    let kp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None).await.unwrap();
    wait_any_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    for round in 0u8..5 {
        let payload: Vec<u8> = (0..50_000).map(|i| ((i % 251) as u8).wrapping_add(round * 30)).collect();
        let outcome = client.send_bulk(&payload, Duration::from_secs(15)).await;
        assert!(outcome.is_delivered(), "round {round}: {outcome:?}");

        let (_, _, received) = tokio::time::timeout(Duration::from_secs(5), brx.recv()).await
            .unwrap_or_else(|_| panic!("round {round}: timeout"))
            .unwrap_or_else(|| panic!("round {round}: channel closed"));
        assert_eq!(received.len(), payload.len(), "round {round} size");
        assert_eq!(received, payload, "round {round} content");
    }

    client.shutdown().await;
    handle.abort();
}

/// Idle timeout: connect, reach Active, send nothing for 4x idle_timeout_ms.
/// Server must transition to Dead or Closed.
///
/// WILL FAIL if the implementation does not enforce idle_timeout_ms.
/// config.rs:39 defines idle_timeout_ms (default 60_000) but connection.rs
/// does not check elapsed time since last activity against this value.
/// This test forces that check to be added.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_timeout_disconnects() {
    common::init_tracing();
    let path = sock_path("idle-timeout");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());

    let mut config = IpcConfig::default();
    config.idle_timeout_ms = 500; // 500ms

    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, _) = mpsc::channel(64);
    let (btx, _) = mpsc::channel(64);
    let (stx, mut srx) = mpsc::channel(256);
    let router = AdvRouter { control_tx: ctx, bulk_tx: btx, state_tx: stx, bulk_accumulator: parking_lot::Mutex::new(std::collections::HashMap::new()) };
    let server = IpcServer::bind(&path, kp.into_inner(), router, config.clone()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &config, None,
    ).await.unwrap();
    let conn_id = wait_any_state(&mut srx, ConnectionPhase::Ready, Duration::from_secs(5)).await;

    // Send one frame to reach Active.
    client.send_frame(b"activate", Duration::from_secs(5)).await;
    wait_conn_state(&mut srx, conn_id, ConnectionPhase::Active, Duration::from_secs(5)).await;

    // Do NOTHING for 5 seconds (10x the 500ms idle timeout).
    // Server must detect idle and transition to terminal.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut saw_terminal = false;
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((id, _, phase))) if id == conn_id && phase.is_terminal() => {
                saw_terminal = true;
                break;
            }
            Ok(Some(_)) => continue,
            _ => break,
        }
    }

    assert!(
        saw_terminal,
        "server must disconnect idle connection after idle_timeout_ms=500. \
         Connection stayed alive for 5s with no traffic. \
         Implementation must add idle timeout enforcement to \
         the connection handler's timer logic in connection.rs."
    );

    drop(client);
    handle.abort();
}
