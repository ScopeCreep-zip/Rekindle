//! An accept settles the request it answers (live A/B findings 56, 60):
//! every receipt of the requester's `FriendRequest` is lost, so only the
//! accepter's `FriendAccept` proves it arrived. It settles the row
//! `delivered`, the row stops heading the requester's handshake stream,
//! and the `FriendsKey` behind it reaches the accepter: presence lights up
//! on both sides, and the request is not sent again after the accept.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::message::SendState;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{
    ChannelMessageEvent, FriendEvent, PresenceEvent, SubscriptionEvent,
};

use crate::harness::{eventually, start, Events, SimNode};
use crate::pair::{accept, has_request_from, is_accepted_friend, key, LIMIT};

/// Whether `events` show `peer` away.
fn sees_away(events: &Events, peer: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some("away"))
    })
}

fn request_delivered(event: &SubscriptionEvent) -> bool {
    matches!(
        event,
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus { client_id, state, .. })
            if client_id.starts_with("FriendRequest:") && *state == SendState::Delivered
    )
}

fn acknowledged(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::RequestAcknowledged { peer_key } => Some(peer_key),
        _ => None,
    }
}

async fn away(node: &SimNode) {
    node.ok(IpcRequest::PresenceSet {
        status: SessionStatus::Away,
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_accept_settles_a_request_whose_receipts_were_lost() {
    let (sim, mut nodes) = start(2, 97).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    // By Ann's friend code, so the request goes by app_call; Ann's
    // answers vanish, so its every receipt is lost.
    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendCode).await).expect("the code");
    sim.fault(Fault::Unanswered {
        node: 1,
        unanswered: true,
    });
    let (rae_events, ann_events) = (nodes[0].events().await, nodes[1].events().await);
    nodes[0]
        .ok(IpcRequest::FriendAddFromCode {
            code: code.url,
            message: "hi".to_string(),
        })
        .await;
    assert!(eventually(&sim, LIMIT, || has_request_from(&nodes[1], &rae)).await);
    accept(&nodes[1], &nodes[0]).await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await);
    assert!(
        eventually(&sim, LIMIT, || async {
            SimNode::saw(&rae_events, request_delivered)
        })
        .await,
        "the accept settled Rae's request delivered"
    );
    assert_eq!(
        rae_events.count(&ann, acknowledged),
        0,
        "no receipt of the request reached Rae: the accept settled it"
    );
    away(&nodes[0]).await;
    away(&nodes[1]).await;
    assert!(
        eventually(&sim, LIMIT * 20, || async { sees_away(&ann_events, &rae) }).await,
        "Rae's FriendsKey left behind the settled request: Ann sees her presence"
    );
    assert!(
        eventually(&sim, LIMIT * 20, || async { sees_away(&rae_events, &ann) }).await,
        "Rae sees Ann's presence"
    );
}
