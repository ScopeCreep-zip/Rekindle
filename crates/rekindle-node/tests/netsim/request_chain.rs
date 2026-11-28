//! A friend request's dm chain record (decision 67 item 2, design §2):
//! the request names the requester's chain record before any consent, so
//! the addressee's node never watches or reads it until she accepts (a
//! vulnerable user's node does not follow a stranger's record because he
//! asked), and the requester forgets the record's writer when the request
//! ends unanswered or the friend is removed, as a removed friend's records
//! are released (`unwatch_friend`).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dht_layout::dm_participant;

use crate::group_dm::own_chain;
use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, request, LIMIT};

/// Long enough for every attempt and watch a request starts to settle.
const SETTLE: Duration = Duration::from_secs(120);

/// Ari (0) asks Bea (1); his chain record for her, named in the request.
async fn asked(seed: u64) -> (rekindle_netsim::Netsim, Vec<SimNode>, String) {
    let (sim, mut nodes) = start(2, seed).await;
    for (node, name) in nodes.iter_mut().zip(["Ari", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    request(&sim, &nodes[0], &nodes[1]).await;
    let chain = own_chain(&nodes[0], &key(&nodes[1])).expect("Ari's request named his chain");
    (sim, nodes, chain)
}

/// Ari's chain `HEAD` is on the network, and Bea's node neither watches
/// nor reads the record.
async fn untouched_by_bea(sim: &rekindle_netsim::Netsim, chain: &str) {
    tokio::time::sleep(SETTLE).await;
    assert!(
        sim.network_get(chain, dm_participant::HEAD).is_some(),
        "Ari published his chain's HEAD before his request went"
    );
    assert!(
        sim.watched_subkeys(1, chain).is_empty(),
        "Bea's node holds no watch on a chain she never consented to"
    );
    assert_eq!(
        sim.app_gets(1, chain),
        0,
        "Bea's node never reads a chain she never consented to"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn request_chain_not_watched_before_accept() {
    let (sim, _nodes, chain) = asked(221).await;
    untouched_by_bea(&sim, &chain).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn request_chain_not_watched_before_accept_declined() {
    let (sim, nodes, chain) = asked(222).await;
    nodes[1]
        .ok(IpcRequest::FriendReject {
            public_key: key(&nodes[0]),
        })
        .await;
    untouched_by_bea(&sim, &chain).await;
}

/// Whether Ari's vault still holds his chain's writer for Bea.
fn ari_holds_writer(nodes: &[SimNode]) -> bool {
    own_chain(&nodes[0], &key(&nodes[1])).is_some()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn request_chain_writer_forgotten_on_cancel() {
    let (sim, nodes, _chain) = asked(223).await;
    nodes[0]
        .ok(IpcRequest::FriendCancelRequest {
            public_key: key(&nodes[1]),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { !ari_holds_writer(&nodes) }).await,
        "a cancelled request's chain writer is forgotten"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn request_chain_writer_forgotten_on_pending_removal() {
    let (sim, nodes, _chain) = asked(224).await;
    nodes[0]
        .ok(IpcRequest::FriendRemove {
            public_key: key(&nodes[1]),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { !ari_holds_writer(&nodes) }).await,
        "removing a friend who never answered forgets the chain writer"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn request_chain_writer_forgotten_on_friend_removal() {
    let (sim, mut nodes) = start(2, 225).await;
    for (node, name) in nodes.iter_mut().zip(["Ari", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    assert!(ari_holds_writer(&nodes), "Ari's request named his chain");
    nodes[0]
        .ok(IpcRequest::FriendRemove {
            public_key: key(&nodes[1]),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { !ari_holds_writer(&nodes) }).await,
        "a removed friend's chain writer is forgotten"
    );
}
