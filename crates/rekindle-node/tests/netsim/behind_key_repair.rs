//! Plan E3.2 S7m item 1c (joiner-driven missing-generation repair): a
//! member out of reach when the community key rotates, so that the
//! rotator's `distribute_mek` push never reaches it, merges the
//! `MEKGenerationBump` once it is back and must then ask for the key it
//! lacks (the D16 `RequestMEK` path) until it holds it. Nobody pushes it
//! again: the rotation is over, so without the ask it stays a generation
//! behind for good and reads nothing sent under the new key.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, Community};

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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_away_at_rotation_asks_for_the_key_it_missed() {
    let community = Community::of(4, 223).await;
    let bea = community.pseudonym(1).await;
    let before = community_generation(&community, 3).await;
    assert!(before > 0, "node 3 holds the community key");
    // Node 3 is cut off from every other member before the kick, so the
    // rotation's push cannot reach it.
    community.sim.fault(Fault::Partition {
        a: vec![0, 1, 2],
        b: vec![3],
    });
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.governance_key.clone(),
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
    assert_eq!(
        community_generation(&community, 3).await,
        before,
        "node 3 was away for the rotation"
    );
    community.sim.fault(Fault::Heal);
    assert!(
        eventually(&community.sim, Duration::from_secs(150), || async {
            community_generation(&community, 3).await > before
        })
        .await,
        "node 3 asks for and receives the key it missed: {}",
        community.mek_list(3).await
    );
    tokio::time::sleep(Duration::from_secs(120)).await;
    community.sim.quiesce().await;
    for i in [0, 2, 3] {
        assert_eq!(
            community_generation(&community, i).await,
            before + 1,
            "node {i}: one kick moves the key one generation: {}",
            community.mek_list(i).await
        );
    }
}
