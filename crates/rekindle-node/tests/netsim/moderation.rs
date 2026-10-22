//! Plan E3.1 F2: a ban takes the member out of every other member's list,
//! and a timeout ends at its time on the sim clock.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_ipc::protocol::{LifecycleRequest, MemberRequest};
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
