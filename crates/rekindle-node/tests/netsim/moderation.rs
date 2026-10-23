//! Plan E3.1 F2: a ban takes the member out of every other member's list,
//! and a timeout ends at its time on the sim clock. Plan E3.2: a kick is
//! the durable `MemberRemoved` op, so it leaves every reader's member set
//! and the kicked member's `RequestMEK` is answered by no one.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelMessageRequest, LifecycleRequest, MemberRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use serde_json::Value;

use crate::harness::{eventually, Community};

/// Node `i`'s view of `pseudonym` in the member list, if listed.
async fn listed(community: &Community, i: usize, pseudonym: &str) -> Option<Value> {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .as_array()
        .and_then(|members| {
            members
                .iter()
                .find(|m| m["pseudonymKey"].as_str() == Some(pseudonym))
                .cloned()
        })
}

/// The key generations node `i` holds, by scope (the age a listing carries
/// moves with the clock).
async fn held_keys(community: &Community, i: usize) -> Vec<(Value, Value)> {
    community
        .mek_list(i)
        .await
        .as_array()
        .map(|keys| {
            keys.iter()
                .map(|k| (k["channel_id"].clone(), k["generation"].clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The creator bans a member: the third member's list loses it, the ban
/// list names it, and a member without `BAN_MEMBERS` is refused.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ban_removes_member() {
    let community = Community::of(3, 83).await;
    let bea = community.pseudonym(1).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            listed(&community, 2, &bea).await.is_some()
        })
        .await,
        "the member is listed before the ban"
    );
    let refused = community.nodes[2]
        .call(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Ban {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
                reason: None,
            },
        )))
        .await
        .expect_err("a member without BAN_MEMBERS is refused");
    assert_eq!(refused.code, 403, "{}", refused.message);

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Ban {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
                reason: Some("spam".into()),
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            listed(&community, 2, &bea).await.is_none()
        })
        .await,
        "the banned member leaves the other member's list"
    );
    let bans = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::BanList {
                community: community.governance_key.clone(),
            },
        )))
        .await;
    assert!(
        bans.as_array()
            .is_some_and(|b| b.iter().any(|v| v["pseudonymKey"].as_str() == Some(&bea))),
        "the ban list names the member: {bans}"
    );
}

/// The creator times a member out for two minutes: the list carries the
/// end time, and once the sim clock passes it the timeout is gone.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn timeout_expires() {
    let community = Community::of(2, 89).await;
    let bea = community.pseudonym(1).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            listed(&community, 0, &bea).await.is_some()
        })
        .await,
        "the member is listed"
    );
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Timeout {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
                duration_seconds: 120,
                reason: None,
            },
        )))
        .await;
    let until = listed(&community, 0, &bea).await.expect("listed")["timeoutUntil"].as_u64();
    assert!(until.is_some(), "the timeout is in force");
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            listed(&community, 0, &bea)
                .await
                .is_some_and(|m| m["timeoutUntil"].is_null())
        })
        .await,
        "the timeout ends at its time"
    );
}

/// The creator kicks a member: the `MemberRemoved` op takes it out of
/// every reader's member set (the creator's and the third member's), and
/// a `RequestMEK` it sends afterwards is refused by every responder, so it
/// gains no key.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn kick_removes_member_and_refuses_its_key_request() {
    let community = Community::of(3, 97).await;
    let bea = community.pseudonym(1).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            listed(&community, 2, &bea).await.is_some()
        })
        .await,
        "the member is listed before the kick"
    );
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
            },
        )))
        .await;
    for reader in [0, 2] {
        assert!(
            eventually(&community.sim, Duration::from_secs(120), || async {
                listed(&community, reader, &bea).await.is_none()
            })
            .await,
            "the kicked member leaves node {reader}'s member set"
        );
    }
    community.sim.quiesce().await;
    let general = community.general_id(1).await;
    let held_before = held_keys(&community, 1).await;
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::MekRequest {
                community: community.governance_key.clone(),
                channel: general,
                generation: 0,
            },
        )))
        .await;
    community.sim.quiesce().await;
    assert_eq!(
        held_keys(&community, 1).await,
        held_before,
        "no responder answers the kicked member's RequestMEK"
    );
    assert!(
        listed(&community, 2, &bea).await.is_none(),
        "the kick holds on the third member"
    );
}
