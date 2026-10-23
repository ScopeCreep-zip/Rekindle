//! A first contact is receipted (live A/B finding 27): the accepter's
//! `FriendAccept` PreKeyInit goes by `app_call` and its row settles only on
//! the receipt. One lost on the way is attempted again until it lands, and
//! one whose receipt is lost is attempted again and answered with the same
//! receipt by a requester that already opened it: one acceptance, one
//! session that carries messages both ways.

use std::time::Duration;

use rekindle_netsim::Fault;
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start};
use crate::pair::{accept, is_accepted_friend, key, request, talk, LIMIT};

fn accepted(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::Accepted { peer_key, .. } => Some(peer_key),
        _ => None,
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_accept_is_attempted_again_until_it_lands() {
    let (sim, mut nodes) = start(2, 61).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let ann = key(&nodes[1]);
    request(&sim, &nodes[0], &nodes[1]).await;
    let events = nodes[0].events().await;
    // Every app message and call is lost while Ann accepts.
    sim.fault(Fault::DropMessages { per_mille: 1000 });
    accept(&nodes[1], &nodes[0]).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(
        !is_accepted_friend(&nodes[0], &ann).await,
        "the accept was lost"
    );
    sim.fault(Fault::DropMessages { per_mille: 0 });
    assert!(
        eventually(&sim, LIMIT * 20, || is_accepted_friend(&nodes[0], &ann)).await,
        "the outbox attempted the accept again and Rae is accepted"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the lost accept").await;
    assert_eq!(events.count(&ann, accepted), 1, "one acceptance");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_re_sent_accept_is_receipted_without_a_second_session() {
    let (sim, mut nodes) = start(2, 63).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let ann = key(&nodes[1]);
    request(&sim, &nodes[0], &nodes[1]).await;
    let events = nodes[0].events().await;
    // Rae's answers are lost: the accept reaches her, its receipt does not.
    sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: true,
    });
    accept(&nodes[1], &nodes[0]).await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "the first attempt opened the session"
    );
    // Ann attempts the unreceipted accept again meanwhile.
    tokio::time::sleep(Duration::from_secs(120)).await;
    sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: false,
    });
    tokio::time::sleep(Duration::from_secs(1800)).await;
    talk(&sim, &nodes[0], &nodes[1], "after the re-sent accept").await;
    assert_eq!(
        events.count(&ann, accepted),
        1,
        "a re-sent init changes nothing"
    );
}
