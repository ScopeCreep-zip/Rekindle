//! Plan FX3 follow-up KP (netsim d): a member locked through a rotation
//! unlocks a generation behind. The Host says so, `KeyStatus` Pending,
//! until a key holder answers its `RequestMEK`; then Current. The post
//! sealed under the key it missed is held while the key is owed, never
//! stored as damaged, so the member reads its text once the key lands.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest, MemberRequest};
use rekindle_types::dto::community::KeyState;
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, PASSPHRASE};

/// The community key generation node `i` holds, or 0 before it holds one.
async fn community_generation(community: &Community, i: usize) -> u64 {
    community
        .mek_list(i)
        .await
        .as_array()
        .into_iter()
        .flatten()
        .filter(|k| k["channel_id"].as_str().is_none_or(str::is_empty))
        .filter_map(|k| k["generation"].as_u64())
        .max()
        .unwrap_or(0)
}

/// Node `i`'s `CommunityView::key_status.state`, as its window reads it.
async fn shown_state(community: &Community, i: usize) -> String {
    let views = community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        )))
        .await;
    views
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|v| v["keyStatus"]["state"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn key_state(event: &SubscriptionEvent, want: KeyState) -> bool {
    matches!(
        event,
        SubscriptionEvent::Governance(GovernanceEvent::KeyStatus { status, .. })
            if status.state == want
    )
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_locked_through_a_rotation_waits_for_the_key_then_reads_the_gap() {
    let community = Community::of(4, 241).await;
    let bea = community.pseudonym(1).await;
    let before = community_generation(&community, 3).await;
    assert!(before > 0, "node 3 holds the community key");
    assert_eq!(shown_state(&community, 3).await, "current");
    let member = &community.nodes[3];
    let key = member.public_key.clone().expect("an identity");
    member.ok(IpcRequest::Lock).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: bea,
                reason: None,
            },
        )))
        .await;
    for i in [0, 2] {
        assert!(
            eventually(&community.sim, Duration::from_secs(180), || async {
                community_generation(&community, i).await > before
            })
            .await,
            "node {i} receives the rotated key"
        );
    }
    assert!(
        community.post(0, "sealed in the gap").await.is_ok(),
        "the creator posts under the new key"
    );
    assert!(
        community
            .reads(2, "sealed in the gap", Duration::from_secs(120))
            .await,
        "a key holder reads the gap post"
    );
    member
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    let events = member.events().await;
    let shown_pending = shown_state(&community, 3).await == "pending";
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community_generation(&community, 3).await > before
        })
        .await,
        "node 3 asks for and receives the key it missed: {}",
        community.mek_list(3).await
    );
    assert!(
        shown_pending || events.any(|e| key_state(e, KeyState::Pending)),
        "node 3 showed its key Pending after the unlock"
    );
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            shown_state(&community, 3).await == "current"
        })
        .await,
        "node 3 shows its key Current once a holder answers"
    );
    assert!(
        events.any(|e| key_state(e, KeyState::Current)),
        "the Host published the Current status"
    );
    assert!(
        community
            .reads(3, "sealed in the gap", Duration::from_secs(120))
            .await,
        "node 3 reads the post sealed under the key it missed"
    );
}
