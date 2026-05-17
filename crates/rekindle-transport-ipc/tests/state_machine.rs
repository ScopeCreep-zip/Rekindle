//! Connection state machine transition legality.
//!
//! Upstream reference: snow/tests/general.rs:105-124 test_noise_state_change
//! Upstream reference: snow/tests/general.rs:696-736 test_checkpointing
//!
//! WILL FAIL if:
//! - on_connection_state_changed fires illegal transitions
//! - A connection goes backward (Active→Ready) outside of recovery
//! - Closed is never reached after graceful shutdown
//! - can_transition_to table is inconsistent with documented transitions

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
        "rekindle-sm-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct SmRouter {
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
    control_tx: mpsc::Sender<(u64, Bytes)>,
}
impl FrameRouter for SmRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) { let _ = self.control_tx.try_send((id, p)); }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, old: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, old, new));
    }
}

/// Collect all transitions for a graceful connect→send→shutdown lifecycle.
/// Assert every (old, new) pair passes can_transition_to().
/// Assert the final state is terminal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_transitions_legal_graceful() {
    common::init_tracing();
    let path = sock_path("sm-graceful");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(1024);
    let (ctx, _) = mpsc::channel(1024);
    let server = IpcServer::bind(&path, kp.into_inner(), SmRouter { state_tx: stx, control_tx: ctx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    let mut transitions: Vec<(u64, ConnectionPhase, ConnectionPhase)> = Vec::new();

    // Wait for Ready.
    let conn_id;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some(t @ (id, _, ConnectionPhase::Ready))) => {
                transitions.push(t);
                conn_id = id;
                break;
            }
            Ok(Some(t)) => { transitions.push(t); }
            _ => panic!("never reached Ready"),
        }
    }

    // Trigger Active.
    client.send_frame(b"trigger-active", Duration::from_secs(5)).await;
    while let Ok(Some(t)) = tokio::time::timeout(Duration::from_millis(500), srx.recv()).await {
        transitions.push(t);
    }

    // Graceful shutdown.
    client.shutdown().await;

    // Collect remaining transitions.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some(t)) => {
                transitions.push(t);
                if t.0 == conn_id && t.2.is_terminal() { break; }
            }
            _ => break,
        }
    }

    let ours: Vec<(u64, ConnectionPhase, ConnectionPhase)> = transitions
        .iter().copied().filter(|t| t.0 == conn_id).collect();
    assert!(!ours.is_empty(), "no transitions recorded for conn {conn_id}");

    for (_, old, new) in &ours {
        assert!(
            old.can_transition_to(*new),
            "illegal transition: {old} → {new} (conn {conn_id})"
        );
    }

    let final_phase = ours.last().unwrap().2;
    assert!(
        final_phase.is_terminal(),
        "final phase: {final_phase}, expected terminal (conn {conn_id})"
    );

    handle.abort();
}

/// Same as above but with hard-drop (no shutdown frame).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_transitions_legal_hard_drop() {
    common::init_tracing();
    let path = sock_path("sm-harddrop");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (stx, mut srx) = mpsc::channel(1024);
    let (ctx, _) = mpsc::channel(1024);
    let server = IpcServer::bind(&path, kp.into_inner(), SmRouter { state_tx: stx, control_tx: ctx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let ckp = keys::generate_keypair().unwrap();
    let client = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    let mut transitions: Vec<(u64, ConnectionPhase, ConnectionPhase)> = Vec::new();

    let conn_id;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some(t @ (id, _, ConnectionPhase::Ready))) => {
                transitions.push(t);
                conn_id = id;
                break;
            }
            Ok(Some(t)) => { transitions.push(t); }
            _ => panic!("never reached Ready"),
        }
    }

    client.send_frame(b"activate", Duration::from_secs(5)).await;
    while let Ok(Some(t)) = tokio::time::timeout(Duration::from_millis(300), srx.recv()).await {
        transitions.push(t);
    }

    drop(client); // hard drop

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some(t)) => {
                transitions.push(t);
                if t.0 == conn_id && t.2.is_terminal() { break; }
            }
            _ => break,
        }
    }

    let ours: Vec<(u64, ConnectionPhase, ConnectionPhase)> = transitions
        .iter().copied().filter(|t| t.0 == conn_id).collect();
    assert!(!ours.is_empty(), "no transitions for conn {conn_id}");

    for (_, old, new) in &ours {
        assert!(
            old.can_transition_to(*new),
            "illegal transition after hard drop: {old} → {new} (conn {conn_id})"
        );
    }

    let final_phase = ours.last().unwrap().2;
    assert!(
        final_phase.is_terminal(),
        "did not reach terminal after hard drop: {final_phase} (conn {conn_id})"
    );

    handle.abort();
}

/// Exhaustive unit test of the can_transition_to table.
/// Verifies every allowed transition and catches any unexpected additions.
#[test]
fn transition_table_exhaustive() {
    common::init_tracing();
    use ConnectionPhase::*;
    let all = [Handshaking, Ready, Active, Degraded, Dead, Draining, Closing, Closed];

    let allowed = vec![
        (Handshaking, Ready),
        (Handshaking, Closed),
        (Ready, Active),
        (Ready, Closed),
        (Active, Degraded),
        (Active, Draining),
        (Active, Dead),
        (Active, Closed),
        (Degraded, Active), // recovery
        (Degraded, Dead),
        (Degraded, Draining),
        (Degraded, Closed),
        (Dead, Closed),
        (Draining, Closing),
        (Draining, Dead),
        (Draining, Closed),
        (Closing, Closed),
    ];

    // Every allowed transition must pass.
    for &(from, to) in &allowed {
        assert!(
            from.can_transition_to(to),
            "{from} → {to} must be allowed but isn't"
        );
    }

    // Count total allowed and verify no extras exist.
    let mut total = 0;
    for &from in &all {
        for &to in &all {
            if from.can_transition_to(to) {
                total += 1;
                assert!(
                    allowed.contains(&(from, to)),
                    "unexpected allowed transition: {from} → {to}"
                );
            }
        }
    }
    assert_eq!(
        total, allowed.len(),
        "transition count mismatch: table has {}, can_transition_to allows {total}",
        allowed.len()
    );

    // No state transitions to Handshaking.
    for &from in &all {
        assert!(!from.can_transition_to(Handshaking), "{from} → Handshaking must be forbidden");
    }

    // Closed cannot transition anywhere.
    for &to in &all {
        assert!(!Closed.can_transition_to(to), "Closed → {to} must be forbidden");
    }
}
