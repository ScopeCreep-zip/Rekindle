//! One community's change rebuilds that community alone (plan E3.1 C1):
//! a member of two communities sees a channel created in one of them; its
//! watch on that governance record rebuilds it (`GovernanceRebuilt`, from
//! the Host on both modes) and the other community is not re-read.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelRequest, LifecycleRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use crate::harness::{eventually, Community};

fn rebuilt(event: &SubscriptionEvent, community: &str) -> bool {
    matches!(
        event,
        SubscriptionEvent::Governance(GovernanceEvent::GovernanceRebuilt { community: c })
            if c == community
    )
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_change_in_one_of_two_communities_rebuilds_only_that_one() {
    let mut community = Community::of(2, 137).await;
    let first = community.governance_key.clone();
    let created = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityCreate {
                name: "Second".into(),
                description: String::new(),
                approval_required: false,
            },
        )))
        .await;
    let second = created["governanceKey"]
        .as_str()
        .expect("a governance key")
        .to_string();
    community.governance_key.clone_from(&second);
    community.join(1).await;
    community.governance_key.clone_from(&first);
    // Every join-time write settles before we watch.
    tokio::time::sleep(Duration::from_secs(60)).await;

    let events = community.nodes[1].events().await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: first.clone(),
                name: "announcements".into(),
                kind: "text".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await;
    let seen = eventually(&community.sim, Duration::from_secs(120), || async {
        events.any(|e| rebuilt(e, &first))
    })
    .await;
    assert!(seen, "the changed community is rebuilt");
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(
        !events.any(|e| rebuilt(e, &second)),
        "the unchanged community is not rebuilt"
    );
}
