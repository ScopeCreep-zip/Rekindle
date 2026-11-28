//! A message queued to a peer whose request we then cancelled (live A/B
//! findings 31, 35): the cancel discards it, so it never reaches the
//! friendship a later request opens and never holds the stream behind it.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::message::SendState;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{accept, holds_message, is_accepted_friend, key, request, send, LIMIT};

async fn own_state(node: &SimNode, peer: &str, body: &str) -> Option<SendState> {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 50,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history
        .iter()
        .find(|m| m.is_own && m.body == body)
        .map(|m| m.send_state)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_message_queued_before_a_cancel_is_discarded_and_holds_nothing() {
    let (sim, mut nodes) = start(2, 71).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    request(&sim, &nodes[0], &nodes[1]).await;
    // Rae writes to Ann while her request is pending: no session yet.
    send(&nodes[0], &ann, "D1").await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    nodes[0]
        .ok(IpcRequest::FriendCancelRequest {
            public_key: ann.clone(),
        })
        .await;
    assert_eq!(
        own_state(&nodes[0], &ann, "D1").await,
        Some(SendState::Failed),
        "the cancel discarded D1"
    );
    request(&sim, &nodes[0], &nodes[1]).await;
    accept(&nodes[1], &nodes[0]).await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await);
    send(&nodes[0], &ann, "D2").await;
    assert!(
        eventually(&sim, LIMIT, || holds_message(&nodes[1], &rae, "D2")).await,
        "D2 is delivered"
    );
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert!(
        !holds_message(&nodes[1], &rae, "D1").await,
        "D1 never reaches the later friendship"
    );
}
