//! A/B finding R3-8, its open half: a member who left on its own keeps its
//! channel slot's keys in any backup of its profile, so a restored backup
//! can write the slot after the leave and back-date the post (its clock an
//! hour behind, so the post's `sent_at` precedes the leave). The leaver
//! signs its slot heads into its `MemberLeft`, and the history keeps a
//! departed author's post only at or below them, never by the time the
//! author wrote into it, so no reader shows or stores the back-dated post,
//! while the leaver's genuine post from before the leave still shows.
//!
//! The backup is taken while the member is locked, before it leaves, and
//! put back after; it is cut off from every peer when it posts, as a
//! restore made off-line (`equivocation.rs` stages a restored backup).

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, Community, PASSPHRASE};
use crate::moderation::listed;

const LIMIT: Duration = Duration::from_secs(120);

async fn unlock(community: &Community, i: usize) {
    let node = &community.nodes[i];
    node.ok(IpcRequest::Unlock {
        identity: node.public_key.clone().expect("an identity"),
        passphrase: PASSPHRASE.into(),
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_leavers_back_dated_slot_write_is_kept_by_no_member() {
    let community = Community::of(3, 163).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let bea = community.pseudonym(1).await;
    let _ = community.post(1, "bea before leaving").await;
    for reader in [0, 2] {
        assert!(
            community.reads(reader, "bea before leaving", LIMIT).await,
            "node {reader} reads the member's post before the leave"
        );
    }
    community.nodes[1].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[1]);
    unlock(&community, 1).await;
    community.nodes[1]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .expect("the leave commits");
    for reader in [0, 2] {
        assert!(
            eventually(&community.sim, LIMIT, || async {
                listed(&community, reader, &bea).await.is_none()
            })
            .await,
            "node {reader} merges the leave"
        );
    }
    community.nodes[1].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[1], &backup);
    community.sim.fault(Fault::Partition {
        a: vec![0, 2],
        b: vec![1],
    });
    unlock(&community, 1).await;
    community.sim.fault(Fault::ClockSkew {
        node: 1,
        ms: -3_600_000,
    });
    community
        .post(1, "bea back-dated before leaving")
        .await
        .expect("the restored backup writes its slot");
    assert!(
        community
            .history_holds(1, "bea back-dated before leaving", LIMIT)
            .await,
        "the post is in the leaver's slot"
    );
    community.sim.fault(Fault::Heal);
    community.sim.quiesce().await;
    for reader in [0, 2] {
        assert!(
            !community
                .reads(reader, "bea back-dated before leaving", LIMIT)
                .await,
            "node {reader} never shows the back-dated post"
        );
        let page = community.history(reader, None, 100).await.expect("history");
        assert!(
            page.iter()
                .all(|m| m["body"].as_str() != Some("bea back-dated before leaving")),
            "node {reader} holds no back-dated post: {page:?}"
        );
        assert!(
            page.iter()
                .any(|m| m["body"].as_str() == Some("bea before leaving")),
            "node {reader} still shows the genuine pre-leave post: {page:?}"
        );
    }
}
