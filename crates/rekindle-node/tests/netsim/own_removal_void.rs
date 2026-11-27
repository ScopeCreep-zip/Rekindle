//! A removal voided while the removed member is logged out (plan E3.2 S9
//! own removal, RM2): the unlock judges each removed community again on
//! the frontiers as published, so the member is restored at unlock rather
//! than kept out by a removal no other member holds.
//!
//! The strong-removal chain of
//! `scenarios::a_removal_concurrent_with_the_removers_own_is_void_everywhere`,
//! with the two kicks made concurrent by a partition: the moderator B
//! (node 1) kicks the member C (node 2) on C's side while the creator A
//! (node 0), cut off from both, kicks B. C merges its removal, is torn
//! down and logs out; the partition heals and A's kick voids B's
//! (p2panda-auth transitive invalidation); then C unlocks.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::Fault;

use crate::equivocation::unlock;
use crate::harness::{eventually, Community};
use crate::moderation::listed;
use crate::scenarios::seat_moderator;

/// The time a governance change takes to reach every member.
const SEEN: Duration = Duration::from_secs(180);

/// Node `by` kicks `target`.
async fn kick(community: &Community, by: usize, target: &str) {
    community.nodes[by]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: target.to_owned(),
                reason: None,
            },
        )))
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_removal_voided_while_logged_out_is_restored_at_unlock() {
    let community = Community::of(3, 1626).await;
    let (bea, cal) = (community.pseudonym(1).await, community.pseudonym(2).await);
    seat_moderator(&community, 1).await;
    assert!(
        eventually(&community.sim, SEEN, || async {
            listed(&community, 1, &cal).await.is_some()
        })
        .await,
        "the moderator lists the member it removes"
    );
    community.sim.quiesce().await;

    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    kick(&community, 1, &cal).await;
    assert!(
        eventually(&community.sim, SEEN, || async {
            community.info(2).await["removed"]["by"].as_str() == Some(bea.as_str())
        })
        .await,
        "the member merges its removal by the moderator: {}",
        community.info(2).await
    );
    community.nodes[2].ok(IpcRequest::Lock).await;
    kick(&community, 0, &bea).await;
    community.sim.fault(Fault::Heal);

    assert!(
        eventually(&community.sim, SEEN, || async {
            listed(&community, 0, &bea).await.is_none()
                && listed(&community, 0, &cal).await.is_some()
        })
        .await,
        "the creator removed the moderator and keeps the member"
    );
    community.sim.quiesce().await;

    unlock(&community, 2).await;
    assert!(
        eventually(&community.sim, SEEN, || async {
            community.info(2).await["removed"].is_null()
        })
        .await,
        "the member is restored at unlock: {}",
        community.info(2).await
    );
    community
        .post(2, "cal after the void")
        .await
        .expect("the restored member posts");
    assert!(
        community.reads(0, "cal after the void", SEEN).await,
        "the creator reads the restored member's post"
    );
}
