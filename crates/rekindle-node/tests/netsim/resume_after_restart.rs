//! Resume after restart (plan E3.1 C2a): a member locks, the community
//! moves on without it (a new channel), and it unlocks again with the
//! network up. The Host's login network stage reopens, hydrates, rebuilds
//! (`GovernanceRebuilt`, with the new channel) and republishes its
//! communities and schedules their history catch-up.
//!
//! Finding (2026-10-22): after the restart the member's MEK is not cached
//! (`ChannelSend` → "MEK generation 0 not cached"), so its posts and reads
//! of what it missed are asserted with the MEK restore (C2b
//! `recover_behind_meks`, E3.5).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, PASSPHRASE};

const LIMIT: Duration = Duration::from_secs(180);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_resumes_its_community_after_a_restart() {
    let community = Community::of(2, 151).await;
    // Every join-time write settles.
    tokio::time::sleep(Duration::from_secs(60)).await;

    let member = &community.nodes[1];
    let key = member.public_key.clone().expect("an identity");
    member.ok(IpcRequest::Lock).await;
    community.nodes[0]
        .ok(IpcRequest::ChannelCreate {
            community: community.governance_key.clone(),
            name: "news".into(),
            kind: "text".into(),
            category: None,
            topic: None,
            slowmode_seconds: 0,
        })
        .await;

    member
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    let events = member.events().await;
    let governance_key = community.governance_key.clone();
    let rebuilt = eventually(&community.sim, LIMIT, || async {
        events.any(|e| {
            matches!(
                e,
                SubscriptionEvent::Governance(GovernanceEvent::GovernanceRebuilt { community: c })
                    if *c == governance_key
            )
        }) && community.info(1).await.to_string().contains("news")
    })
    .await;
    assert!(
        rebuilt,
        "the community is rebuilt from the DHT, with the new channel"
    );
}
