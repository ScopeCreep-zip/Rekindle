//! A request the target refuses ends (live A/B finding 74): the target
//! blocked the requester, so it answers the request's `app_call` with a
//! bare NAK, a judged refusal (RV5). The requester's row settles at once:
//! the pending-out friend shows the refusal, not retrying, and no further
//! attempt goes out over five minutes. A fresh `friend add` clears the
//! error and sends one new request, refused the same way.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Netsim;
use rekindle_types::dto::friends::{Friend, FriendCodeView};
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{friends, key, LIMIT};

const REFUSED: &str = "they did not accept a request from you";

/// The app calls from node `from` delivered to node `to` so far.
fn calls(sim: &Netsim, from: usize, to: usize) -> usize {
    let entry = format!("{to}<-{from} AppCall");
    sim.trace().of(to).filter(|e| *e == entry).count()
}

fn failed(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::HandshakeFailed { peer_key, reason } if reason == REFUSED => Some(peer_key),
        _ => None,
    }
}

async fn row(node: &SimNode, peer: &str) -> Friend {
    friends(node)
        .await
        .into_iter()
        .find(|f| f.public_key == peer)
        .expect("the pending-out friend")
}

async fn add(node: &SimNode, code: &FriendCodeView) {
    node.ok(IpcRequest::FriendAddFromCode {
        code: code.url.clone(),
        message: "hi".to_string(),
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_blocked_requesters_request_is_refused_once_and_shown() {
    let (sim, mut nodes) = start(2, 74).await;
    for (node, name) in nodes.iter_mut().zip(["Mac", "Pop"]) {
        node.unlock_new_identity(name).await;
    }
    let (mac, pop) = (key(&nodes[0]), key(&nodes[1]));
    nodes[1]
        .ok(IpcRequest::FriendBlock {
            public_key: mac.clone(),
        })
        .await;
    let code: FriendCodeView =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendCode).await).expect("the code");
    let events = nodes[0].events().await;
    add(&nodes[0], &code).await;
    assert!(
        eventually(&sim, LIMIT, || async { events.count(&pop, failed) == 1 }).await,
        "the refusal is shown"
    );
    let friend = row(&nodes[0], &pop).await;
    assert_eq!(friend.friendship_state, FriendshipState::PendingOut);
    assert_eq!(friend.handshake_error.as_deref(), Some(REFUSED));
    assert!(!friend.handshake_retrying);
    let attempts = calls(&sim, 0, 1);
    assert_eq!(attempts, 1, "the request went once");
    eventually(&sim, Duration::from_secs(300), || async { false }).await;
    assert_eq!(calls(&sim, 0, 1), attempts, "no attempt after the refusal");
    assert_eq!(events.count(&pop, failed), 1);

    // A fresh add replaces the row and sends one new request.
    add(&nodes[0], &code).await;
    let friend = row(&nodes[0], &pop).await;
    assert_eq!(
        friend.handshake_error, None,
        "the fresh add clears the error"
    );
    assert!(
        eventually(&sim, LIMIT, || async { events.count(&pop, failed) == 2 }).await,
        "the new request is refused the same way"
    );
    eventually(&sim, Duration::from_secs(300), || async { false }).await;
    assert_eq!(calls(&sim, 0, 1), attempts + 1, "one new attempt");
}
