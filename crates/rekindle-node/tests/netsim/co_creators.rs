//! The creator-set round end to end (plan E3.2 S7s.3, lead decisions (16),
//! (17), (20), (23)): a creator names friends as co-creators; the
//! community is written only once every one has accepted, declined or
//! been removed, and its GenesisCert lists the accepts alone; an accepted
//! co-creator joins through its link and holds creator rights; a cancel,
//! a removal and a creator that dies before the genesis each withdraw the
//! invites, and a lock releases the reservation's records.

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_ipc::protocol::{CommunityRequest, LifecycleRequest};
use rekindle_netsim::Netsim;
use rekindle_types::dto::community::{CoCreationSnapshot, CoCreatorState, HeldCreatorInvite};
use rekindle_types::subscription_events::{FriendEvent, GovernanceEvent, SubscriptionEvent};

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
    create_named(creator, "Co", co_creators).await
}

/// Node `creator` starts creating `name` with `co_creators`; returns the
/// reserved anchor.
async fn create_named(creator: &SimNode, name: &str, co_creators: &[&SimNode]) -> String {
    let created = creator
        .ok(lifecycle(LifecycleRequest::CommunityCreate {
            name: name.into(),
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

/// Whether `node` lists `anchor` through the query the GUI's community
/// list reads (`get_community_details` → `CommunityViews`).
async fn views(node: &SimNode, anchor: &str) -> bool {
    node.call(lifecycle(LifecycleRequest::CommunityViews))
        .await
        .is_ok_and(|v| v.to_string().contains(anchor))
}

/// Whether `events` hold the community list's arrival event for `anchor`
/// (`GovernanceEvent::MetadataChanged`, from `CommunityCreated` on a
/// create and `CommunityJoined` on a join).
fn arrived(events: &Events, anchor: &str) -> bool {
    events.governance().iter().any(
        |e| matches!(e, GovernanceEvent::MetadataChanged { community, .. } if community == anchor),
    )
}

/// Lead decision (41): `node` lists `anchor` and heard its arrival, as a
/// direct create or join leaves it.
async fn listed_as_direct(sim: &Netsim, node: &SimNode, events: &Events, anchor: &str) {
    assert!(
        eventually(sim, Duration::from_secs(300), || views(node, anchor)).await,
        "the community list's query lists the community"
    );
    assert!(
        eventually(sim, LIMIT, || async { arrived(events, anchor) }).await,
        "the community list heard the community arrive"
    );
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
    // Lead decision (41): the creator and the co-creator heard the same
    // arrival a direct create and a direct join publish (below).
    listed_as_direct(&sim, ann, &ann_seen, &anchor).await;
    listed_as_direct(&sim, bob, &bob_seen, &anchor).await;
    assert!(!arrived(&cat_seen, &anchor), "Cat heard no arrival");
}

/// Lead decision (41), the reference the co-created community is held
/// to: a direct create and a direct join through a link publish the
/// arrival the community list uses, and both list it through its query.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_direct_create_and_join_publish_the_arrival_a_co_creation_does() {
    let (sim, nodes) = friends_of_zero(2, 9305).await;
    let (ann, bob) = (&nodes[0], &nodes[1]);
    let (ann_seen, bob_seen) = (ann.events().await, bob.events().await);
    let anchor = create_with(ann, &[]).await;
    listed_as_direct(&sim, ann, &ann_seen, &anchor).await;
    let invite = ann
        .ok(lifecycle(LifecycleRequest::InviteCreate {
            community: anchor.clone(),
            max_uses: 1,
            expires_seconds: None,
        }))
        .await;
    let link = invite["url"].as_str().expect("an invite link").to_string();
    bob.ok(lifecycle(LifecycleRequest::CommunityJoin { invite: link }))
        .await;
    listed_as_direct(&sim, bob, &bob_seen, &anchor).await;
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

/// Live finding CC2: right after a co-creation with Bob completes (Bob
/// accepted; his join may still be running), Ann starts a second one naming Bob. Bob holds the
/// new invite, hears `CreatorInvited` for its anchor, and his snapshot
/// lists it, as for the first.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_second_co_creation_after_a_completed_one_reaches_the_same_friend() {
    let (sim, nodes) = friends_of_zero(2, 9306).await;
    let (ann, bob) = (&nodes[0], &nodes[1]);
    let (ann_seen, bob_seen) = (ann.events().await, bob.events().await);
    let first = create_with(ann, &[bob]).await;
    assert!(eventually(&sim, LIMIT, || async { invited(&bob_seen, &first) }).await);
    answer(bob, &first, true).await;
    assert!(
        eventually(&sim, LIMIT, || async { finished(&ann_seen, &first, true) }).await,
        "the first co-creation finishes"
    );
    // At once, as live: Bob's join of the first may still be running.
    let second = create_named(ann, "Co2", &[bob]).await;
    assert_ne!(first, second, "a new reservation");
    assert!(
        eventually(&sim, LIMIT, || async { invited(&bob_seen, &second) }).await,
        "Bob hears the second invite"
    );
    let held = snapshot(bob).await;
    assert_eq!(
        held.invites,
        vec![HeldCreatorInvite {
            anchor: second.clone(),
            peer_key: key(ann),
            community_name: "Co2".into(),
        }],
        "Bob holds the second invite"
    );
    assert!(
        eventually(&sim, Duration::from_secs(300), || lists(bob, &first)).await,
        "Bob joins the first through his link"
    );
}

/// Lead decision (56): a frame's long-running consequence never holds the
/// next frame from the same friend. Ann's `CreatorJoin` for the first
/// co-creation starts Bob's join (many DHT round trips); Ann's next
/// `CreatorInvite`, on the same chain right behind it, is held and shown
/// while that join still runs, not after it (live CC2: ~18 s behind).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_second_creator_frame_is_applied_while_the_first_join_runs() {
    let (sim, nodes) = friends_of_zero(2, 9307).await;
    let (ann, bob) = (&nodes[0], &nodes[1]);
    let (ann_seen, bob_seen) = (ann.events().await, bob.events().await);
    let first = create_with(ann, &[bob]).await;
    assert!(eventually(&sim, LIMIT, || async { invited(&bob_seen, &first) }).await);
    answer(bob, &first, true).await;
    assert!(
        eventually(&sim, LIMIT, || async { finished(&ann_seen, &first, true) }).await,
        "the first co-creation finishes"
    );
    let second = create_named(ann, "Co2", &[bob]).await;
    assert!(
        eventually(&sim, LIMIT, || async { invited(&bob_seen, &second) }).await,
        "Bob hears the second invite"
    );
    assert!(
        !arrived(&bob_seen, &first),
        "the second invite is applied while Bob's join of the first still runs"
    );
    listed_as_direct(&sim, bob, &bob_seen, &first).await;
}

fn ours(node: &SimNode, anchor: &str) -> String {
    (node.host.governance().community_membership(anchor))
        .and_then(|m| m.my_pseudonym_hex)
        .expect("our pseudonym")
}

/// Lead decision (46): Ann and Bob co-create; Cat, a friend of Ann only,
/// joins and is offered the community by Ann. Cat's first accept asks Bob
/// inside the community; Bob answers the request; Cat's next accept
/// creates the successor (its creators Cat and Bob, linked from the op
/// log); Bob is offered it and follows through his sealed invite.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_two_creator_transfer_links_the_co_creator_through_the_community() {
    let (sim, nodes) = friends_of_zero(3, 9311).await;
    let (ann, bob, cat) = (&nodes[0], &nodes[1], &nodes[2]);
    let bob_seen = bob.events().await;
    let anchor = create_with(ann, &[bob]).await;
    assert!(eventually(&sim, LIMIT, || async { invited(&bob_seen, &anchor) }).await);
    answer(bob, &anchor, true).await;
    assert!(eventually(&sim, LIMIT, || async { lists(bob, &anchor).await }).await);
    let invite = ann
        .ok(lifecycle(LifecycleRequest::InviteCreate {
            community: anchor.clone(),
            max_uses: 1,
            expires_seconds: None,
        }))
        .await;
    let url = invite["url"].as_str().expect("an invite link").to_string();
    cat.ok(lifecycle(LifecycleRequest::CommunityJoin { invite: url }))
        .await;
    let cat_old = ours(cat, &anchor);
    let counted = eventually(&sim, LIMIT, || async {
        (ann.host.governance().governance_state(&anchor)).is_some_and(|s| {
            s.is_counted_member(&rekindle_types::id::PseudonymKey::from_hex_lossy(&cat_old))
        })
    })
    .await;
    assert!(counted, "Ann counts Cat");
    ann.ok(lifecycle(LifecycleRequest::CommunityOfferTransfer {
        anchor: anchor.clone(),
        transfer_to: cat_old.clone(),
    }))
    .await;
    let accept = || {
        lifecycle(LifecycleRequest::CommunityAcceptTransfer {
            anchor: anchor.clone(),
        })
    };
    let first = eventually(&sim, LIMIT, || async { cat.call(accept()).await.is_ok() }).await;
    assert!(first, "Cat's first accept asks the co-creator");
    // Bob is told by a backend event that the request waits (the lead's
    // (b) on decision (46)), not by reading the community's state.
    let asked = std::cell::RefCell::new(String::new());
    let told = eventually(&sim, LIMIT, || async {
        bob_seen.any(|e| match e {
            SubscriptionEvent::Governance(GovernanceEvent::SuccessorLinkRequested {
                community,
                request,
                recipient,
                ..
            }) if *community == anchor && *recipient == cat_old => {
                *asked.borrow_mut() = request.clone();
                true
            }
            _ => false,
        })
    })
    .await;
    assert!(told, "Bob is told Cat's link request waits");
    let request = asked.into_inner();
    let snapshot = bob
        .ok(lifecycle(LifecycleRequest::CommunitySuccessors {
            anchor: anchor.clone(),
        }))
        .await;
    assert_eq!(
        snapshot["linkRequests"][0]["request"].as_str(),
        Some(request.as_str()),
        "the successor snapshot lists it for a window opened later"
    );
    bob.ok(lifecycle(LifecycleRequest::CommunityAnswerSuccessorLink {
        anchor: anchor.clone(),
        request,
    }))
    .await;
    let found = std::cell::RefCell::new(String::new());
    let done = eventually(&sim, Duration::from_secs(300), || async {
        let Ok(v) = cat.call(accept()).await else {
            return false;
        };
        let finished = v["pendingCoCreators"].as_array().is_some_and(Vec::is_empty)
            && v["registryKey"].as_str().is_some_and(|k| !k.is_empty());
        if finished {
            *found.borrow_mut() = v["anchor"].as_str().unwrap_or_default().to_owned();
        }
        finished
    })
    .await;
    assert!(done, "Cat's accept completes once Bob linked");
    let new_anchor = found.into_inner();
    let offered = eventually(&sim, Duration::from_secs(300), || async {
        (bob.call(lifecycle(LifecycleRequest::CommunitySuccessors {
            anchor: anchor.clone(),
        }))
        .await
        .ok())
        .is_some_and(|v| v.to_string().contains(&new_anchor))
    })
    .await;
    assert!(offered, "Bob is offered the successor");
    bob.ok(lifecycle(LifecycleRequest::CommunityFollowSuccessor {
        anchor: anchor.clone(),
        new_anchor: new_anchor.clone(),
    }))
    .await;
    assert!(
        eventually(&sim, Duration::from_secs(120), || async {
            lists(bob, &new_anchor).await
        })
        .await
    );
    let creators = creators(&info(bob, &new_anchor).await);
    assert_eq!(
        creators.len(),
        2,
        "the successor's creators are Cat and Bob"
    );
}
