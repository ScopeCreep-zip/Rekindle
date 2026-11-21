//! The creator-set round end to end (plan E3.2 S7s.3, lead decisions (16),
//! (17), (20), (23)): a creator names friends as co-creators; the
//! community is written only once every one has accepted, declined or
//! been removed, and its GenesisCert lists the accepts alone; an accepted
//! co-creator joins through its link and holds creator rights; a cancel,
//! a removal and a creator that dies before the genesis each withdraw the
//! invites, and a lock releases the reservation's records.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_ipc::protocol::{CommunityRequest, LifecycleRequest};
use rekindle_netsim::Netsim;
use rekindle_types::dto::community::{CoCreationSnapshot, CoCreatorState, HeldCreatorInvite};
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start, Events, SimNode, PASSPHRASE};
use crate::pair::{befriend, key, LIMIT};

fn lifecycle(request: LifecycleRequest) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(request))
}

/// `n` nodes with identities; node 0 befriends every other node.
async fn friends_of_zero(n: usize, seed: u64) -> (Netsim, Vec<SimNode>) {
    let (sim, mut nodes) = start(n, seed).await;
    for (i, node) in nodes.iter_mut().enumerate() {
        node.unlock_new_identity(&format!("creator{i}")).await;
    }
    for i in 1..n {
        befriend(&sim, &nodes[0], &nodes[i]).await;
    }
    (sim, nodes)
}

/// Node `creator` starts creating "Co" with `co_creators`; returns the
/// reserved anchor.
async fn create_with(creator: &SimNode, co_creators: &[&SimNode]) -> String {
    let created = creator
        .ok(lifecycle(LifecycleRequest::CommunityCreate {
            name: "Co".into(),
            description: String::new(),
            approval_required: false,
            co_creators: co_creators.iter().map(|n| key(n)).collect(),
        }))
        .await;
    let created: rekindle_types::dto::community::CommunityCreated =
        serde_json::from_value(created).expect("the reserved creation");
    assert_eq!(
        created.pending_co_creators.len(),
        co_creators.len(),
        "every co-creator is waited on"
    );
    created.anchor
}

async fn lists(node: &SimNode, anchor: &str) -> bool {
    node.call(lifecycle(LifecycleRequest::CommunityList))
        .await
        .is_ok_and(|v| v.to_string().contains(anchor))
}

async fn info(node: &SimNode, anchor: &str) -> serde_json::Value {
    node.ok(lifecycle(LifecycleRequest::CommunityInfo {
        anchor: anchor.to_string(),
    }))
    .await
}

fn creators(info: &serde_json::Value) -> Vec<String> {
    serde_json::from_value(info["creator_pseudonyms"].clone()).expect("the creator set")
}

fn invited(events: &Events, anchor: &str) -> bool {
    events
        .friend()
        .iter()
        .any(|e| matches!(e, FriendEvent::CreatorInvited { anchor: a, .. } if a == anchor))
}

fn withdrawn(events: &Events, anchor: &str) -> bool {
    events
        .friend()
        .iter()
        .any(|e| matches!(e, FriendEvent::CreatorInviteWithdrawn { anchor: a, .. } if a == anchor))
}

fn status_of(events: &Events, peer: &str, anchor: &str, status: &str) -> bool {
    events.friend().iter().any(|e| {
        matches!(e, FriendEvent::CoCreatorStatus { peer_key, anchor: a, status: s }
            if peer_key == peer && a == anchor && s == status)
    })
}

fn finished(events: &Events, anchor: &str, created: bool) -> bool {
    events.friend().iter().any(|e| {
        matches!(e, FriendEvent::CoCreationFinished { anchor: a, created: c }
            if a == anchor && *c == created)
    })
}

async fn snapshot(node: &SimNode) -> CoCreationSnapshot {
    serde_json::from_value(
        node.ok(lifecycle(LifecycleRequest::CoCreationSnapshot))
            .await,
    )
    .expect("the co-creation snapshot")
}

async fn answer(node: &SimNode, anchor: &str, accept: bool) {
    node.ok(lifecycle(LifecycleRequest::CreatorInviteAnswer {
        anchor: anchor.to_string(),
        accept,
    }))
    .await;
}

/// Decision (16): Ann names Bob and Cat; Bob accepts and the creation
/// still waits on Cat; Cat declines, the community is written listing Ann
/// and Bob only, Bob joins through his link and his pseudonym is in the
/// creator set (decision (17)), and Ann saw both answers.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_accept_is_listed_a_decline_omitted_and_the_creation_waits_for_both() {
    let (sim, nodes) = friends_of_zero(3, 9301).await;
    let (ann, bob, cat) = (&nodes[0], &nodes[1], &nodes[2]);
    let (ann_seen, bob_seen, cat_seen) =
        (ann.events().await, bob.events().await, cat.events().await);
    let anchor = create_with(ann, &[bob, cat]).await;
    assert!(eventually(&sim, LIMIT, || async { invited(&bob_seen, &anchor) }).await);
    assert!(eventually(&sim, LIMIT, || async { invited(&cat_seen, &anchor) }).await);
    answer(bob, &anchor, true).await;
    let bob_key = key(bob);
    assert!(
        eventually(&sim, LIMIT, || async {
            status_of(&ann_seen, &bob_key, &anchor, "accepted")
        })
        .await,
        "Ann sees Bob's accept"
    );
    assert!(!lists(ann, &anchor).await, "the creation waits on Cat");
    // A window opened now reads what the events said (lead decision (33)).
    let cat_key = key(cat);
    let held = snapshot(cat).await;
    assert_eq!(
        held.invites,
        vec![HeldCreatorInvite {
            anchor: anchor.clone(),
            peer_key: key(ann),
            community_name: "Co".into(),
        }],
        "Cat's held invite"
    );
    let ours = snapshot(ann).await;
    assert_eq!(ours.creations.len(), 1, "Ann's pending creation");
    let state = |peer: &str, status: &str| CoCreatorState {
        peer_key: peer.to_owned(),
        status: status.to_owned(),
    };
    assert_eq!(
        ours.creations[0].co_creators,
        vec![state(&bob_key, "accepted"), state(&cat_key, "pending")],
        "each co-creator's state, in the order named"
    );
    assert!(!finished(&ann_seen, &anchor, true), "not finished yet");
    answer(cat, &anchor, false).await;
    assert!(
        eventually(&sim, LIMIT, || async {
            status_of(&ann_seen, &cat_key, &anchor, "declined")
        })
        .await,
        "Ann sees Cat's decline"
    );
    assert!(
        eventually(&sim, LIMIT, || lists(ann, &anchor)).await,
        "Ann's community is written"
    );
    assert!(
        eventually(&sim, LIMIT, || async { finished(&ann_seen, &anchor, true) }).await,
        "Ann is told the co-creation finished"
    );
    assert!(snapshot(ann).await.creations.is_empty(), "nothing pending");
    let set = creators(&info(ann, &anchor).await);
    assert_eq!(set.len(), 2, "the cert lists Ann and Bob only: {set:?}");
    assert!(
        eventually(&sim, Duration::from_secs(300), || lists(bob, &anchor)).await,
        "Bob joins through his link"
    );
    let bob_pseudonym = info(bob, &anchor).await["our_pseudonym"]
        .as_str()
        .expect("Bob's pseudonym")
        .to_string();
    assert!(
        set.contains(&bob_pseudonym),
        "Bob is a creator from genesis"
    );
    assert!(!lists(cat, &anchor).await, "Cat declined and is no member");
}

/// Decision (16): Bob never answers; Ann removes him, he is sent
/// `CreatorWithdrawn`, and the community is written with Ann alone.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_removed_non_replier_is_withdrawn_and_the_creation_finishes() {
    let (sim, nodes) = friends_of_zero(2, 9302).await;
    let (ann, bob) = (&nodes[0], &nodes[1]);
    let bob_seen = bob.events().await;
    let anchor = create_with(ann, &[bob]).await;
    assert!(eventually(&sim, LIMIT, || async { invited(&bob_seen, &anchor) }).await);
    assert!(!lists(ann, &anchor).await, "the creation waits on Bob");
    ann.ok(lifecycle(LifecycleRequest::CreatorRemove {
        anchor: anchor.clone(),
        identity: key(bob),
    }))
    .await;
    assert!(
        eventually(&sim, LIMIT, || lists(ann, &anchor)).await,
        "the creation finishes"
    );
    assert_eq!(creators(&info(ann, &anchor).await).len(), 1, "Ann alone");
    assert!(
        eventually(&sim, LIMIT, || async { withdrawn(&bob_seen, &anchor) }).await,
        "Bob's invite is withdrawn"
    );
}

/// A cancel releases the reservation (its anchor closed on Ann's node)
/// and withdraws every invite; no community is written.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_cancel_releases_the_records_and_withdraws_every_invite() {
    let (sim, nodes) = friends_of_zero(3, 9303).await;
    let (ann, bob, cat) = (&nodes[0], &nodes[1], &nodes[2]);
    let (bob_seen, cat_seen) = (bob.events().await, cat.events().await);
    let anchor = create_with(ann, &[bob, cat]).await;
    assert!(eventually(&sim, LIMIT, || async { invited(&bob_seen, &anchor) }).await);
    assert!(sim.is_open(0, &anchor), "the reservation holds the anchor");
    ann.ok(lifecycle(LifecycleRequest::CommunityCreateCancel {
        anchor: anchor.clone(),
    }))
    .await;
    assert!(!sim.is_open(0, &anchor), "the cancel released the anchor");
    for seen in [&bob_seen, &cat_seen] {
        assert!(eventually(&sim, LIMIT, || async { withdrawn(seen, &anchor) }).await);
    }
    assert!(!lists(ann, &anchor).await, "no community is written");
}

/// Decisions (23), (20): Ann logs out while her creation waits on Bob:
/// the lock releases the reservation's records at once; after a restart
/// her unlock finds the row and Bob gets `CreatorWithdrawn`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_creator_restarted_before_genesis_withdraws_at_its_unlock() {
    let (sim, mut nodes) = friends_of_zero(2, 9304).await;
    let bob_seen = nodes[1].events().await;
    let bob_key = key(&nodes[1]);
    let anchor = create_with(&nodes[0], &[&nodes[1]]).await;
    assert!(eventually(&sim, LIMIT, || async { invited(&bob_seen, &anchor) }).await);
    assert!(sim.is_open(0, &anchor), "the reservation holds the anchor");
    nodes[0].ok(IpcRequest::Logout).await;
    assert!(
        !sim.is_open(0, &anchor),
        "the lock released the reservation's anchor"
    );
    assert!(
        !withdrawn(&bob_seen, &anchor),
        "nothing is withdrawn before the unlock"
    );
    let ann = nodes.remove(0).restart(&sim, Duration::from_secs(30)).await;
    ann.ok(IpcRequest::Unlock {
        identity: key(&ann),
        passphrase: PASSPHRASE.into(),
    })
    .await;
    assert!(
        eventually(&sim, LIMIT, || async { withdrawn(&bob_seen, &anchor) }).await,
        "Bob ({bob_key}) gets CreatorWithdrawn at Ann's unlock"
    );
    assert!(
        !lists(&ann, &anchor).await,
        "the dead creation is never written"
    );
}
