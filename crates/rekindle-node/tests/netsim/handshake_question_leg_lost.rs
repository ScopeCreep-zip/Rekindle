//! A first contact's call path lost on the wire (plan RV2, lead decision
//! 66): the requester's request carries a reply inbox, the accepter writes
//! its `FriendAccept` PreKeyInit there as well as by `app_call`, and the
//! requester opens the durable copy through the same admission. With every
//! call to the requester lost (`Fault::CallHangs`: the question never
//! reaches it, Veilid 0.5.8 `rpc_processor/mod.rs:2038-2104`), it completes
//! the friendship from its reply inbox and rotates its General route once
//! (evidence source 6); with both paths up, one acceptance and no rotation.

use std::time::Duration;

use rekindle_netsim::Fault;
use rekindle_records::port::RouteClass;
use rekindle_route::liveness::{HINT_WINDOW, REPORT_ROTATE_AGE};
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
async fn the_requester_completes_from_its_reply_inbox_and_rotates_once() {
    let (sim, mut nodes) = start(2, 71).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let ann = key(&nodes[1]);
    request(&sim, &nodes[0], &nodes[1]).await;
    // Rae's General route is old enough for Ann to hold its blob.
    tokio::time::sleep(REPORT_ROTATE_AGE + Duration::from_secs(10)).await;
    let general = nodes[0].routes.own_id(RouteClass::General);
    assert!(general.is_some(), "Rae's General route is up");
    let events = nodes[0].events().await;
    // Every call to Rae is lost on the wire.
    sim.fault(Fault::CallHangs {
        node: 0,
        hangs: true,
    });
    accept(&nodes[1], &nodes[0]).await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "Rae opened Ann's accept from her reply inbox"
    );
    sim.fault(Fault::CallHangs {
        node: 0,
        hangs: false,
    });
    tokio::time::sleep(HINT_WINDOW + Duration::from_secs(5)).await;
    let rotated = nodes[0].routes.own_id(RouteClass::General);
    assert!(
        rotated.is_some() && rotated != general,
        "no call copy came over General: it rotated"
    );
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(
        nodes[0].routes.own_id(RouteClass::General),
        rotated,
        "one reply inbox write rotates once"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the durable accept").await;
    assert_eq!(events.count(&ann, accepted), 1, "one acceptance");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn both_paths_up_accept_once_and_rotate_nothing() {
    let (sim, mut nodes) = start(2, 73).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let ann = key(&nodes[1]);
    request(&sim, &nodes[0], &nodes[1]).await;
    tokio::time::sleep(REPORT_ROTATE_AGE + Duration::from_secs(10)).await;
    let general = nodes[0].routes.own_id(RouteClass::General);
    let events = nodes[0].events().await;
    accept(&nodes[1], &nodes[0]).await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "Rae is accepted"
    );
    // Past the window a lone durable copy rotates on (evidence source 6),
    // and short of the inbound-silence trigger.
    tokio::time::sleep(HINT_WINDOW + Duration::from_secs(5)).await;
    assert_eq!(
        nodes[0].routes.own_id(RouteClass::General),
        general,
        "the call copy came over General: no evidence"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the accept").await;
    assert_eq!(
        events.count(&ann, accepted),
        1,
        "one acceptance, two copies"
    );
}
