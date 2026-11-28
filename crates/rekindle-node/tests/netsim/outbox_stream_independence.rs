//! Plan E2.5j: one stream's unanswered call does not hold another
//! stream's send. Ida removes Kim, so her presence key rotates and its
//! `FriendsKey` goes to Jon as a `friend_handshake` row by `app_call`;
//! Jon's replies are lost, so that call waits out its timeout
//! (`CALL_TIMEOUT_MS`). A message Ida sends Jon meanwhile is on his
//! `dm` stream, and its hint reaches him within the hint bound, not after
//! the handshake call (at the base the pump awaited each stream's attempt
//! in turn, and the hint left only once the call timed out).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::MESSAGE;
use rekindle_netsim::Fault;
use rekindle_types::message::CALL_TIMEOUT_MS;

use crate::harness::{eventually, start};
use crate::pair::{befriend, holds_message, key, send, talk};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unanswered_handshake_call_does_not_hold_a_dm_to_the_same_peer() {
    let (sim, mut nodes) = start(3, 131).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon", "Kim"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    befriend(&sim, &nodes[0], &nodes[2]).await;
    talk(&sim, &nodes[0], &nodes[1], "warm").await;
    let (ida, kim) = (key(&nodes[0]), key(&nodes[2]));
    // Every reply of Jon's is lost: a call to him waits out its timeout.
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    nodes[0]
        .ok(IpcRequest::FriendRemove { public_key: kim })
        .await;
    send(&nodes[0], &key(&nodes[1]), "meanwhile").await;
    // Two message spans: well inside the handshake call's timeout.
    let bound = MESSAGE.max * 2;
    assert!(bound < Duration::from_millis(CALL_TIMEOUT_MS));
    assert!(
        eventually(&sim, bound, || holds_message(&nodes[1], &ida, "meanwhile")).await,
        "Jon holds the message within {bound:?}, the handshake call still waiting"
    );
}
