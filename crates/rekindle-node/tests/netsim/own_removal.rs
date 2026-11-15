//! A/B finding R1-1 (plan E3.2 S9 own removal): a kicked member reacts to
//! its own removal once it merges the `MemberRemoved` naming it. It
//! announces `MembershipEvent::Kicked` naming the remover, marks the
//! community removed in `community info` (by whom), forgets the
//! community's keys, refuses its own sends, and keeps its history; the
//! other members see nothing new from it.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::harness::{eventually, Community};
use crate::moderation::listed;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kicked_member_leaves_locally_and_keeps_its_history() {
    let community = Community::of(3, 163).await;
    let ann = community.pseudonym(0).await;
    let bea = community.pseudonym(1).await;
    let bea_events = community.nodes[1].events().await;
    community
        .post(1, "bea before the kick")
        .await
        .expect("a member posts");
    assert!(
        community
            .reads(0, "bea before the kick", Duration::from_secs(120))
            .await,
        "the creator reads the member's post"
    );
    assert!(
        !community
            .mek_list(1)
            .await
            .as_array()
            .is_none_or(Vec::is_empty),
        "the member holds the community's keys before the kick"
    );

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community.info(1).await["removed"]["by"].as_str() == Some(ann.as_str())
        })
        .await,
        "the kicked member's info shows it removed by the creator: {}",
        community.info(1).await
    );
    community.sim.quiesce().await;
    assert!(
        bea_events.any(|e| matches!(
            e,
            SubscriptionEvent::Membership(MembershipEvent::Kicked {
                target_pseudonym,
                removed_by: Some(by),
                ..
            }) if *target_pseudonym == bea && *by == ann
        )),
        "the kicked member announces its own removal, naming the remover"
    );
    assert_eq!(
        community.info(1).await["removed"]["banned"].as_bool(),
        Some(false),
        "a kick, not a ban"
    );
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            community
                .mek_list(1)
                .await
                .as_array()
                .is_none_or(Vec::is_empty)
        })
        .await,
        "the kicked member forgot the community's keys: {}",
        community.mek_list(1).await
    );
    let refused = community
        .post(1, "bea after the kick")
        .await
        .expect_err("a removed member's send is refused");
    assert_eq!(refused.code, 403, "{}", refused.message);
    assert!(
        community
            .history_holds(1, "bea before the kick", Duration::from_secs(30))
            .await,
        "the kicked member keeps its history"
    );

    // Long past a presence beat and a keepalive round: the others see
    // nothing new of it.
    tokio::time::sleep(Duration::from_secs(120)).await;
    community.sim.quiesce().await;
    for reader in [0, 2] {
        assert!(
            listed(&community, reader, &bea).await.is_none(),
            "node {reader} does not list the kicked member again"
        );
        assert!(
            !community
                .reads(reader, "bea after the kick", Duration::from_secs(10))
                .await,
            "node {reader} shows nothing new of the kicked member"
        );
    }
}
