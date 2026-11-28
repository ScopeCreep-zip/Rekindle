//! A friend request to a peer we cannot reach (plan E2.5, D22): adding by
//! invite returns once the request is committed to the outbox, nothing
//! reaches the peer or comes back while they are cut off, and once they
//! are reachable the outbox sends it, its `app_call` receipt raises
//! `RequestAcknowledged`, and they hold the request.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start};
use crate::pair::{has_request_from, key, LIMIT};

fn acknowledged(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::RequestAcknowledged { peer_key } => Some(peer_key),
        _ => None,
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_request_to_an_unreachable_peer_is_queued_and_sent_later() {
    let (sim, mut nodes) = start(2, 53).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    // By Ann's friend code: the request goes by app_call, which a
    // partition stops (an invite's request is a record write instead).
    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendCode).await).expect("the code");
    sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1],
    });
    let events = nodes[0].events().await;
    nodes[0]
        .ok(IpcRequest::FriendAddFromCode {
            code: code.url,
            message: "hi".to_string(),
        })
        .await;

    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(
        !has_request_from(&nodes[1], &rae).await,
        "nothing got through"
    );
    assert_eq!(events.count(&ann, acknowledged), 0, "no receipt yet");

    sim.fault(Fault::Heal);
    assert!(
        eventually(&sim, LIMIT * 10, || has_request_from(&nodes[1], &rae)).await,
        "the outbox sent the request once Ann was reachable"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            events.count(&ann, acknowledged) == 1
        })
        .await,
        "its receipt settled the row"
    );
}

/// An invite redeemed while our DHT is unreachable: the add returns once
/// the redemption is committed, nothing lands in the inviter's inbox while
/// we are offline, and the outbox writes it once we are back.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_offline_redemption_is_committed_and_lands_after_the_heal() {
    let (sim, mut nodes) = start(2, 57).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let rae = key(&nodes[0]);
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    nodes[0]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(
        !has_request_from(&nodes[1], &rae).await,
        "nothing landed while Rae was offline"
    );
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: false,
    });
    assert!(
        eventually(&sim, LIMIT * 10, || has_request_from(&nodes[1], &rae)).await,
        "the outbox wrote the redemption once Rae was back"
    );
}
