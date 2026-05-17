//! Per-client frame ordering proof under concurrent load.
//!
//! Upstream reference: snow/tests/vectors.rs:199-253 confirm_message_vectors
//! which iterates messages in strict order and asserts each matches expected.
//!
//! WILL FAIL if:
//! - The IO loop's select! branches reorder frames within a single connection
//! - The server processes frames out of the order they arrived on one socket
//! - The read_task → control_loop channel drops or reorders messages

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
        "rekindle-ord-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct OrderRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
}
impl FrameRouter for OrderRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) {
        let _ = self.control_tx.try_send((id, p));
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
}

/// 4 clients × 100 frames. Per-client ordering MUST be preserved.
/// Cross-client interleaving is expected and acceptable.
///
/// Each client sends "N:MMMMM" where N=client index, MMMMM=zero-padded seq.
/// After all 400 frames arrive, extract each client's subsequence and verify
/// monotonically increasing sequence numbers.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn per_client_ordering_4x100() {
    common::init_tracing();
    let path = sock_path("order-4x100");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let kp = keys::generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, mut crx) = mpsc::channel(4096);
    let server = IpcServer::bind(&path, kp.into_inner(), OrderRouter { control_tx: ctx }, IpcConfig::default()).unwrap();
    let handle = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;

    let mut tasks = Vec::new();
    for cidx in 0u8..4 {
        let path = path.clone();
        tasks.push(tokio::spawn(async move {
            let ckp = keys::generate_keypair().unwrap();
            let c = IpcClient::connect(
                Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
            ).await.unwrap();
            for i in 0u32..100 {
                let msg = format!("{cidx}:{i:05}");
                let outcome = c.send_frame(msg.as_bytes(), Duration::from_secs(5)).await;
                assert!(outcome.is_delivered(), "c{cidx} f{i}: {outcome:?}");
            }
            c.shutdown().await;
        }));
    }
    for t in tasks { t.await.unwrap(); }

    let mut frames: Vec<String> = Vec::new();
    while let Ok(Some((_, p))) = tokio::time::timeout(Duration::from_millis(500), crx.recv()).await {
        frames.push(String::from_utf8(p.to_vec()).unwrap());
    }
    assert_eq!(frames.len(), 400, "expected 400 frames, got {}", frames.len());

    for cidx in 0u8..4 {
        let prefix = format!("{cidx}:");
        let seqs: Vec<u32> = frames.iter()
            .filter(|f| f.starts_with(&prefix))
            .map(|f| f[prefix.len()..].parse().unwrap())
            .collect();
        assert_eq!(seqs.len(), 100, "client {cidx}: expected 100, got {}", seqs.len());
        for w in seqs.windows(2) {
            assert!(
                w[0] < w[1],
                "client {cidx} out of order: frame {} arrived before {}",
                w[0], w[1]
            );
        }
    }

    handle.abort();
}
