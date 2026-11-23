//! A first contact's call path lost on the wire (plan RV2, lead decision
//! 66): the requester's request carries a reply inbox, the accepter writes
//! its `FriendAccept` PreKeyInit there as well as by `app_call`, and the
//! requester opens the durable copy through the same admission. With every
//! call to the requester lost (`Fault::CallHangs`: the question never
//! reaches it, Veilid 0.5.8 `rpc_processor/mod.rs:2038-2104`), it completes
//! the friendship from its reply inbox and rotates its General route once
//! (evidence source 6). With both paths up the call's receipt settles the
//! row first and the write's wait is dropped
//! (`first_contact_durable::attempt`): one acceptance, no rotation. With
//! the call stalled on its way (`Fault::CallStalls`) the durable copy lands
//! first and the call copy follows within `HINT_WINDOW`: one acceptance,
//! and the call copy withdraws the evidence (`InboundLiveness::call_seen`),
//! so nothing rotates.

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_netsim::{Fault, Netsim};
use rekindle_records::port::{RecordSchema, RouteClass};
use rekindle_route::liveness::{HINT_WINDOW, REPORT_ROTATE_AGE};
use rekindle_types::dht_layout::request_inbox;
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
async fn both_paths_up_the_call_settles_first_and_nothing_rotates() {
    let (sim, mut nodes) = start(2, 73).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let ann = key(&nodes[1]);
    let before = inboxes_watched_by(&sim, 0);
    request(&sim, &nodes[0], &nodes[1]).await;
    let replies: BTreeSet<String> = inboxes_watched_by(&sim, 0)
        .difference(&before)
        .cloned()
        .collect();
    assert_eq!(
        replies.len(),
        1,
        "Rae watches the reply inbox of her open request"
    );
    tokio::time::sleep(REPORT_ROTATE_AGE + Duration::from_secs(10)).await;
    let general = nodes[0].routes.own_id(RouteClass::General);
    let events = nodes[0].events().await;
    accept(&nodes[1], &nodes[0]).await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "Rae is accepted"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            inboxes_watched_by(&sim, 0).is_disjoint(&replies)
        })
        .await,
        "the settled request released its reply inbox and the watch on it"
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
        "one acceptance"
    );
}

/// How long Ann's call to Rae is stuck before it is sent: long enough for
/// her reply inbox write to reach Rae first, well inside [`HINT_WINDOW`].
const CALL_STALL: Duration = Duration::from_secs(6);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_durable_copy_first_then_the_call_copy_rotates_nothing() {
    let (sim, mut nodes) = start(2, 75).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let ann = key(&nodes[1]);
    request(&sim, &nodes[0], &nodes[1]).await;
    tokio::time::sleep(REPORT_ROTATE_AGE + Duration::from_secs(10)).await;
    let general = nodes[0].routes.own_id(RouteClass::General);
    assert!(general.is_some(), "Rae's General route is up");
    let events = nodes[0].events().await;
    // Ann's call is slow on its way to Rae, not lost: her write lands first.
    sim.fault(Fault::CallStalls {
        node: 0,
        stall: Some(CALL_STALL),
    });
    accept(&nodes[1], &nodes[0]).await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "Rae is accepted"
    );
    // Past the window a lone durable copy would rotate (evidence source 6).
    tokio::time::sleep(HINT_WINDOW + Duration::from_secs(5)).await;
    sim.fault(Fault::CallStalls {
        node: 0,
        stall: None,
    });
    assert_eq!(
        nodes[0].routes.own_id(RouteClass::General),
        general,
        "the call copy came over General within the window: no evidence"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the accept").await;
    assert_eq!(
        events.count(&ann, accepted),
        1,
        "one acceptance, two copies"
    );
}

/// The request-inbox-shaped records (one owner subkey, one one-time
/// writer) the node watches: an open request's reply inbox among them.
fn inboxes_watched_by(sim: &Netsim, node: usize) -> BTreeSet<String> {
    sim.watched_by(node)
        .into_iter()
        .filter(|(_, schema)| {
            matches!(schema, RecordSchema::Smpl { owner_subkeys, members }
                if *owner_subkeys == request_inbox::OWNER_SUBKEY_COUNT && members.len() == 1)
        })
        .map(|(key, _)| key)
        .collect()
}
