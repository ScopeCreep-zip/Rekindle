//! A peer's handshake frames wait only for what they need (plan HOL1, lead
//! decision 47): the accepter's `FriendAccept` reaches the requester, but
//! the requester's answer to that app call is lost (`Fault::Unanswered`:
//! the question is delivered and processed, only the reply never comes
//! back). The accepter's receipted row is attempted again and again, yet
//! a group invite to the same peer, which needs neither the friendship nor
//! the session, lands on its first attempt, and the accepter's presence
//! key, which needs both, goes once the requester's first frame under the
//! session settles the first contact (libsignal's implicit acknowledgement).

use std::time::Duration;

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};
use rekindle_netsim::Fault;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::group_dm::view;
use crate::harness::{eventually, start, Events};
use crate::pair::{accept, befriend, holds_message, is_accepted_friend, key, request, send, LIMIT};

/// Whether `events` showed `peer` online: only a holder of `peer`'s
/// presence key opens their status.
fn saw_online(events: &Events, peer: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some("online"))
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_accept_answer_holds_back_only_the_frames_that_need_it() {
    let (sim, mut nodes) = start(3, 147).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[2], &nodes[0]).await;
    let (ann, bea) = (key(&nodes[0]), key(&nodes[1]));
    let events = nodes[1].events().await;
    request(&sim, &nodes[1], &nodes[0]).await;
    // Bea takes Ann's accept, but every answer of hers is lost from here
    // to the end: Ann's FriendAccept row never hears its receipt.
    sim.fault(Fault::Unanswered {
        node: 1,
        unanswered: true,
    });
    accept(&nodes[0], &nodes[1]).await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[1], &ann)).await);
    // Two minutes with the accept unreceipted: Ann's row is attempted
    // again over this span, its answers lost each time.
    tokio::time::sleep(Duration::from_secs(120)).await;

    let gid: String = serde_json::from_value(
        nodes[0]
            .ok(IpcRequest::GroupDm(GroupDmRequest::Create {
                name: "raid night".to_string(),
                members: vec![bea.clone(), key(&nodes[2])],
                origin_peer: None,
            }))
            .await,
    )
    .expect("the group is created");
    // One message round trip and the invite's own work, well inside the
    // outbox's first backoff: the invite goes on its first attempt, not
    // behind the unreceipted accept.
    assert!(
        eventually(&sim, Duration::from_secs(10), || async {
            view(&nodes[1], &gid).await.is_some_and(|v| v.pending)
        })
        .await,
        "Bea holds the group invite from Ann's first attempt"
    );

    // Bea's first frame under the session reaches Ann (Ann answers; only
    // Bea's answers are lost), settling Ann's first contact delivered.
    send(&nodes[1], &ann, "hi Ann").await;
    assert!(eventually(&sim, LIMIT, || holds_message(&nodes[0], &bea, "hi Ann")).await);
    assert!(
        eventually(&sim, Duration::from_secs(30), || async {
            saw_online(&events, &ann)
        })
        .await,
        "Ann's presence key reaches Bea within one retry of Bea's first frame"
    );
}
