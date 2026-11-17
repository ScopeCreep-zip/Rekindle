//! Live A/B findings F7b-1 and F7b-2 (2026-11-17), Backend Owns Policy:
//! every frontend gets each membership verdict once, from the merged
//! governance, with its full payload. A kick reaches the kicked member and
//! the others as one `Kicked` naming the remover and carrying the reason
//! (the gossip echo, which carries neither, used to arrive first); a leave
//! reaches the leaver's own frontends and every other member as one
//! `Left`, whether or not the departure gossip reached them. A join goes
//! out at once with no name, and the joiner's verified card names it once
//! read (plan E3.2 F9-2).

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest, MemberRequest};
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events};
use crate::moderation::listed;

/// Why the creator kicks the member.
const REASON: &str = "f7b reason";

/// How many `Kicked` events about `target` `events` holds, and how many of
/// them name `by` and carry [`REASON`].
fn kicked(events: &Events, target: &str, by: &str) -> (usize, usize) {
    let about = |e: &SubscriptionEvent, full: bool| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::Kicked {
            target_pseudonym, removed_by, reason, ..
        }) if target_pseudonym == target
            && (!full || (removed_by.as_deref() == Some(by) && reason.as_deref() == Some(REASON))))
    };
    (
        events.matching(|e| about(e, false)),
        events.matching(|e| about(e, true)),
    )
}

/// How many `Left` events about `leaver` `events` holds.
fn left(events: &Events, leaver: &str) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::Left { pseudonym, .. })
            if pseudonym == leaver)
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kick_is_one_kicked_with_its_reason_on_every_member() {
    let community = Community::of(3, 717).await;
    let ann = community.pseudonym(0).await;
    let bea = community.pseudonym(1).await;
    let bea_events = community.nodes[1].events().await;
    let cid_events = community.nodes[2].events().await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
                reason: Some(REASON.into()),
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community.info(1).await["removed"]["by"].as_str() == Some(ann.as_str())
                && listed(&community, 2, &bea).await.is_none()
        })
        .await,
        "the kick merges on the kicked member and on the third"
    );
    // Long past the gossip echo and a rebuild round.
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.sim.quiesce().await;
    for (node, events) in [(1, &bea_events), (2, &cid_events)] {
        assert_eq!(
            kicked(events, &bea, &ann),
            (1, 1),
            "node {node} gets exactly one Kicked, naming the remover and the reason"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_leave_is_one_left_on_the_leaver_and_every_member() {
    let community = Community::of(3, 719).await;
    let bea = community.pseudonym(1).await;
    let bea_events = community.nodes[1].events().await;
    let cid_events = community.nodes[2].events().await;
    community.nodes[1]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .expect("the leave commits");
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            listed(&community, 2, &bea).await.is_none()
        })
        .await,
        "the leave merges on the third member"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.sim.quiesce().await;
    assert_eq!(
        left(&bea_events, &bea),
        1,
        "the leaver's own frontends see its leave once"
    );
    assert_eq!(
        left(&cid_events, &bea),
        1,
        "another member sees the leave once"
    );
}

/// Plan E3.2 F9-2 (A/B round 9 saw `displayName: ""`): another member gets
/// one `Joined` with no name, never "", and then one `MemberCardChanged`
/// with the name on the joiner's verified card.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_is_named_by_the_joiners_card_once_read() {
    let mut community = Community::create(2, 721).await;
    let ann_events = community.nodes[0].events().await;
    community.join(1).await;
    let bea = community.pseudonym(1).await;
    let card = |e: &SubscriptionEvent| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::MemberCardChanged {
            pseudonym, display_name, ..
        }) if *pseudonym == bea && display_name == "member1")
    };
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            ann_events.any(card)
        })
        .await,
        "the joiner's card names it on the creator"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.sim.quiesce().await;
    let joined = |named: bool| {
        ann_events.matching(|e| {
            matches!(e, SubscriptionEvent::Membership(MembershipEvent::Joined {
                pseudonym, display_name, ..
            }) if *pseudonym == bea && display_name.is_some() == named)
        })
    };
    assert_eq!(
        (joined(false), joined(true)),
        (1, 0),
        "one Joined, before the card is read, with no name"
    );
    assert_eq!(
        ann_events.matching(card),
        1,
        "the card's name is announced once"
    );
}
