//! Plan E3.2 S9: the verdicts the merged governance holds beyond joins and
//! removals reach every member's frontends, the target's own included,
//! once each, from the merge: a timeout and its lifting (`TimedOut`,
//! `TimeoutRemoved`, each with its `TimeoutStatusChanged`), and a
//! permission change (`EffectivePermissionsChanged` for the member whose
//! permissions moved, `ChannelPermissionsChanged` for an overwrite). None of
//! these had a producer after S8 retired the unsigned wire arms, so a
//! window showed a remote timeout only after a re-read.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest, MemberRequest};
use rekindle_types::subscription_events::{GovernanceEvent, MembershipEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events};

/// Why the creator times the member out.
const REASON: &str = "s9 timeout";
/// `permissions::SEND_MESSAGES`.
const SEND_MESSAGES: u64 = 1 << 16;

fn timed_out(events: &Events, target: &str) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::TimedOut {
            target_pseudonym, duration_seconds: 3600, reason, ..
        }) if target_pseudonym == target && reason.as_deref() == Some(REASON))
    })
}

fn timeout_removed(events: &Events, target: &str) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::TimeoutRemoved {
            target_pseudonym, ..
        }) if target_pseudonym == target)
    })
}

/// `TimeoutStatusChanged` events about `target`: (with an end, lifted).
fn status_changed(events: &Events, target: &str) -> (usize, usize) {
    let about = |until: bool| {
        events.matching(|e| {
            matches!(e, SubscriptionEvent::Membership(MembershipEvent::TimeoutStatusChanged {
                pseudonym, timeout_until, ..
            }) if pseudonym == target && timeout_until.is_some() == until)
        })
    };
    (about(true), about(false))
}

fn permissions_changed(events: &Events, member: &str) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Governance(GovernanceEvent::EffectivePermissionsChanged {
            member: m, ..
        }) if m == member)
    })
}

fn channel_changed(events: &Events, channel: &str) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Governance(GovernanceEvent::ChannelPermissionsChanged {
            channel: c, ..
        }) if c == channel)
    })
}

async fn members(community: &Community, request: MemberRequest) {
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(request)))
        .await;
}

/// Wait until both `events` hold `pick` at least once, then let gossip
/// echoes and a rebuild round pass.
async fn settle(community: &Community, events: [&Events; 2], pick: impl Fn(&Events) -> usize) {
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            events.iter().all(|e| pick(e) > 0)
        })
        .await,
        "the change merges on the target and on the third member"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.sim.quiesce().await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_timeout_and_its_lifting_reach_every_member_once() {
    let community = Community::of(3, 733).await;
    let bea = community.pseudonym(1).await;
    let both = [
        &community.nodes[1].events().await,
        &community.nodes[2].events().await,
    ];
    members(
        &community,
        MemberRequest::Timeout {
            community: community.anchor.clone(),
            target_pseudonym: bea.clone(),
            duration_seconds: 3600,
            reason: Some(REASON.into()),
        },
    )
    .await;
    settle(&community, both, |e| timed_out(e, &bea)).await;
    for (node, events) in [(1, both[0]), (2, both[1])] {
        assert_eq!(
            timed_out(events, &bea),
            1,
            "node {node}: one TimedOut with its reason"
        );
        assert_eq!(status_changed(events, &bea), (1, 0), "node {node}: one end");
        assert_eq!(
            permissions_changed(events, &bea),
            1,
            "node {node}: the timeout strips the member's permissions once"
        );
    }
    members(
        &community,
        MemberRequest::TimeoutRemove {
            community: community.anchor.clone(),
            target_pseudonym: bea.clone(),
        },
    )
    .await;
    settle(&community, both, |e| timeout_removed(e, &bea)).await;
    for (node, events) in [(1, both[0]), (2, both[1])] {
        assert_eq!(
            timeout_removed(events, &bea),
            1,
            "node {node}: one TimeoutRemoved"
        );
        assert_eq!(
            status_changed(events, &bea),
            (1, 1),
            "node {node}: one lift"
        );
        assert_eq!(
            permissions_changed(events, &bea),
            2,
            "node {node}: restored once"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_overwrite_is_one_channel_and_one_member_permission_change() {
    let community = Community::of(3, 737).await;
    let bea = community.pseudonym(1).await;
    let cat = community.pseudonym(2).await;
    let general = community.general_id(0).await;
    let both = [
        &community.nodes[1].events().await,
        &community.nodes[2].events().await,
    ];
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelOverwriteSet {
                community: community.anchor.clone(),
                channel_id: general.clone(),
                target_type: "member".into(),
                target_id: bea.clone(),
                allow: 0,
                deny: SEND_MESSAGES,
            },
        )))
        .await;
    settle(&community, both, |e| channel_changed(e, &general)).await;
    for (node, events) in [(1, both[0]), (2, both[1])] {
        assert_eq!(
            channel_changed(events, &general),
            1,
            "node {node}: one channel change"
        );
        assert_eq!(
            permissions_changed(events, &bea),
            1,
            "node {node}: the denied member's permissions change once"
        );
        assert_eq!(
            permissions_changed(events, &cat),
            0,
            "node {node}: a member the overwrite does not name is untouched"
        );
    }
}
