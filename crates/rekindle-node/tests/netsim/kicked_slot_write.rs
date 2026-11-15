//! A/B finding R2-4 (plan E3.2, A/B round 1): a removed member's post that
//! reaches the readers through the channel record (its slot stays writable
//! until E3.3) passes the same D26 membership gate as its gossip copy, so
//! no member stores or shows it, not even as undecryptable.
//!
//! The member is cut off from the others while it is kicked, so it posts
//! before it can learn its removal; the post's gossip is lost to the
//! partition and its slot write reaches the readers once it heals, by the
//! record's watch and by the history read.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::Fault;

use crate::harness::Community;
use crate::moderation::listed;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kicked_members_slot_write_is_stored_by_no_member() {
    let community = Community::of(3, 151).await;
    let bea = community.pseudonym(1).await;
    community.sim.fault(Fault::Partition {
        a: vec![0, 2],
        b: vec![1],
    });
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
            crate::harness::eventually(&community.sim, Duration::from_secs(120), || async {
                listed(&community, reader, &bea).await.is_none()
            })
            .await,
            "node {reader} merges the kick"
        );
    }
    let _ = community.post(1, "bea from the cut-off side").await;
    community.sim.fault(Fault::Heal);
    community.sim.quiesce().await;
    for reader in [0, 2] {
        assert!(
            !community
                .reads(
                    reader,
                    "bea from the cut-off side",
                    Duration::from_secs(120)
                )
                .await,
            "node {reader} never shows the removed member's post"
        );
        let page = community.history(reader, None, 100).await.expect("history");
        assert!(
            page.iter()
                .all(|m| m["senderId"].as_str() != Some(bea.as_str())),
            "node {reader} holds no entry of the removed member: {page:?}"
        );
    }
}
