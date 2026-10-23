//! A friendship started again after a removal the peer never heard of
//! (live A/B finding 38): Rae removes Ann while no frame reaches Ann, so
//! Rae's `Unfriended` waits for its ack; Ann asks again and Rae accepts.
//! The new friendship settles the stale `Unfriended` (and every other
//! handshake row of the ended friendship), so it never reaches Ann and
//! the friendship stands.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use crate::harness::{eventually, start};
use crate::pair::{accept, befriend, is_accepted_friend, key, request, talk, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_new_friendship_settles_the_ended_ones_unfriended() {
    let (sim, mut nodes) = start(2, 97).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    let ann_events = nodes[1].events().await;
    // No frame reaches Ann: Rae's Unfriended stays pending.
    sim.fault(Fault::DropMessages { per_mille: 1000 });
    nodes[0]
        .ok(IpcRequest::FriendRemove {
            public_key: ann.clone(),
        })
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    // Ann asks again (through the invite's inbox) and Rae accepts.
    request(&sim, &nodes[1], &nodes[0]).await;
    accept(&nodes[0], &nodes[1]).await;
    sim.fault(Fault::DropMessages { per_mille: 0 });
    assert!(
        eventually(&sim, LIMIT * 4, || is_accepted_friend(&nodes[1], &rae)).await,
        "Ann holds the new friendship"
    );
    // The stale row was settled by the new friendship: none is left for
    // a user's Discard to find.
    assert!(
        nodes[0]
            .call(IpcRequest::OutboxDiscard {
                client_id: format!("unfriended:{ann}"),
            })
            .await
            .is_err(),
        "the ended friendship's Unfriended no longer waits"
    );
    // Long past every resend of the stale Unfriended.
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert!(
        is_accepted_friend(&nodes[0], &ann).await,
        "Rae still holds Ann"
    );
    assert!(
        is_accepted_friend(&nodes[1], &rae).await,
        "Ann still holds Rae"
    );
    assert!(
        !ann_events.any(|event| matches!(event,
            SubscriptionEvent::Friend(FriendEvent::Removed { peer_key }) if *peer_key == rae)),
        "no stale Unfriended reached Ann"
    );
    talk(&sim, &nodes[0], &nodes[1], "after").await;
}
