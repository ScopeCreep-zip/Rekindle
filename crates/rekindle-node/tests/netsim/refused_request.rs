//! A blocked requester learns nothing (LV23 review, amending RV5 R1 and
//! live A/B finding 74): the blocker answers the request's `app_call` with
//! the silent receipt, the reply a reachable peer who has not answered
//! gives, and keeps nothing of it. The requester's side is the same as for
//! a request to a peer who never accepts: one call, receipted, the friend
//! pending-out with no error, no failure, and no attempt after.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Netsim;
use rekindle_types::dto::friends::{Friend, FriendCodeView};
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{friends, has_request_from, key, LIMIT};

/// The app calls from node `from` delivered to node `to` so far.
fn calls(sim: &Netsim, from: usize, to: usize) -> usize {
    let entry = format!("{to}<-{from} AppCall");
    sim.trace().of(to).filter(|e| *e == entry).count()
}

fn acknowledged(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::RequestAcknowledged { peer_key } => Some(peer_key),
        _ => None,
    }
}

fn failed(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::HandshakeFailed { peer_key, .. } => Some(peer_key),
        _ => None,
    }
}

/// What the requester can see of its request to `peer`.
async fn seen(node: &SimNode, peer: &str) -> (FriendshipState, Option<String>, bool) {
    let friend: Friend = friends(node)
        .await
        .into_iter()
        .find(|f| f.public_key == peer)
        .expect("the pending-out friend");
    (
        friend.friendship_state,
        friend.handshake_error,
        friend.handshake_retrying,
    )
}

async fn code(node: &SimNode) -> FriendCodeView {
    serde_json::from_value(node.ok(IpcRequest::FriendCode).await).expect("the code")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_blocked_requesters_request_is_receipted_and_dropped_silently() {
    let (sim, mut nodes) = start(3, 74).await;
    for (node, name) in nodes.iter_mut().zip(["Mac", "Pop", "Lin"]) {
        node.unlock_new_identity(name).await;
    }
    let (mac, pop, lin) = (key(&nodes[0]), key(&nodes[1]), key(&nodes[2]));
    nodes[1]
        .ok(IpcRequest::FriendBlock {
            public_key: mac.clone(),
        })
        .await;
    let (blocker, other) = (code(&nodes[1]).await, code(&nodes[2]).await);
    let requester = nodes[0].events().await;
    let blocker_events = nodes[1].events().await;
    for code in [&blocker, &other] {
        nodes[0]
            .ok(IpcRequest::FriendAddFromCode {
                code: code.url.clone(),
                message: "hi".to_string(),
            })
            .await;
    }
    assert!(
        eventually(&sim, LIMIT, || async {
            requester.count(&pop, acknowledged) == 1 && requester.count(&lin, acknowledged) == 1
        })
        .await,
        "both requests are receipted"
    );
    // Five simulated minutes for any retry to show.
    eventually(&sim, Duration::from_secs(300), || async { false }).await;

    // The blocker kept nothing and raised nothing.
    assert!(!has_request_from(&nodes[1], &mac).await, "nothing stored");
    assert!(
        !blocker_events.any(|e| matches!(
            e,
            SubscriptionEvent::Friend(FriendEvent::RequestReceived { from_key, .. }) if *from_key == mac
        )),
        "no event"
    );
    assert!(
        has_request_from(&nodes[2], &mac).await,
        "the other holds it"
    );

    // The requester cannot tell the block from a peer who never accepts.
    let to_blocker = seen(&nodes[0], &pop).await;
    assert_eq!(to_blocker, (FriendshipState::PendingOut, None, false));
    assert_eq!(to_blocker, seen(&nodes[0], &lin).await);
    assert_eq!(calls(&sim, 0, 1), 1, "one call, no retry");
    assert_eq!(calls(&sim, 0, 1), calls(&sim, 0, 2));
    assert_eq!(requester.count(&pop, failed), 0);
    assert_eq!(requester.count(&lin, failed), 0);
}
