//! A first contact that cannot open, and the user's Retry (plan E2.4 B7,
//! audit 2026-10-20 F-1): Ann accepts Rae's request while Rae's prekey
//! record shows no signed prekey. Ann's accept fails visibly
//! (`FriendEvent::HandshakeFailed`, her row's `handshake_error`), nothing
//! points at a session reset, and once the record reads again Ann's
//! `FriendRetryHandshake` opens the session and Rae's side is accepted.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::dht_layout::prekeys::HEAD;
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start};
use crate::pair::{accept, friends, is_accepted_friend, key, request, talk, LIMIT};

fn failed(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::HandshakeFailed { peer_key, .. } => Some(peer_key),
        _ => None,
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_accept_opens_on_retry() {
    let (sim, mut nodes) = start(2, 47).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    request(&sim, &nodes[0], &nodes[1]).await;

    // Rae's record shows no signed prekey, and she cannot rewrite it.
    let record = nodes[0]
        .prekey_status()
        .await
        .record_key
        .expect("Rae's prekey record");
    let owner = sim.owner_keypair(&record).expect("its owner keypair");
    let head = sim.network_value(&record, HEAD).expect("her head").data;
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    sim.forge_write(&record, HEAD, Vec::new(), &owner)
        .expect("an owner write");

    let ann_events = nodes[1].events().await;
    accept(&nodes[1], &nodes[0]).await;
    assert!(
        eventually(&sim, LIMIT, || async {
            ann_events.count(&rae, failed) == 1
        })
        .await,
        "the failed accept is visible"
    );
    let row = friends(&nodes[1])
        .await
        .into_iter()
        .find(|f| f.public_key == rae)
        .expect("Rae's row");
    let reason = row.handshake_error.expect("the row says why");
    assert!(
        !reason.contains("Reset"),
        "no session reset for a first contact: {reason}"
    );
    assert!(!is_accepted_friend(&nodes[0], &ann).await);

    // The record reads again; Retry opens the session.
    sim.forge_write(&record, HEAD, head, &owner)
        .expect("an owner write");
    sim.fault(Fault::Heal);
    nodes[1]
        .ok(IpcRequest::FriendRetryHandshake {
            public_key: rae.clone(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "the retried accept reaches Rae"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the retry").await;
    let row = friends(&nodes[1])
        .await
        .into_iter()
        .find(|f| f.public_key == rae)
        .expect("Rae's row");
    assert_eq!(row.handshake_error, None, "the failure is cleared");
}
