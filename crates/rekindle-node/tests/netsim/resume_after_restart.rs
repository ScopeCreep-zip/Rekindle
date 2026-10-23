//! Resume after restart (plan E3.1 C2a): a member locks, the community
//! moves on without it (a new channel), and it unlocks again with the
//! network up. The Host's login network stage reopens, hydrates, rebuilds
//! (`GovernanceRebuilt`, with the new channel) and republishes its
//! communities and schedules their history catch-up; the unlock's MEK
//! stage (plan E3.1 C2c) reloads the stored keys, so the member posts
//! again and reads the message it missed. Its restart writes nothing
//! early (live findings 24, 25): no route or status write to a record the
//! own-records stage does not yet hold, no presence row without a route.

use std::time::Duration;

use rekindle_ipc::protocol::ChannelRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
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
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: community.governance_key.clone(),
                name: "news".into(),
                kind: "text".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await;
    community
        .post(0, "while you were away")
        .await
        .expect("the creator posts");

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
    assert!(
        community.reads(1, "while you were away", LIMIT).await,
        "the member reads the message it missed"
    );
    community
        .post(1, "back again")
        .await
        .expect("the member posts after the restart");
    assert!(
        community.reads(0, "back again", LIMIT).await,
        "the creator reads the member's post"
    );
    for early in ["not held writable", "our_route_blob is None"] {
        let seen = crate::logs::warned(early);
        assert!(seen.is_empty(), "an early write warned: {seen:#?}");
    }
}
