//! A/B finding R4-1 (plan E3.5, rotation trigger idempotence): one kick
//! is one departure, so it moves the community key exactly one generation
//! on every remaining member. The remover queues the departure rotation
//! when it writes the `MemberRemoved`, and the `Kick` notice it gossips
//! comes back to it relayed by the mesh (and reaches every other member,
//! more than once when relayed): none of those copies may rotate a
//! departure that was already rotated for.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};

use crate::harness::{eventually, Community};

/// The community key generation node `i` holds (the entry with no
/// channel), or 0 before it holds one.
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
async fn one_kick_rotates_the_key_once() {
    let community = Community::of(4, 211).await;
    let bea = community.pseudonym(1).await;
    let remaining = [0, 2, 3];
    let mut before = Vec::new();
    for i in remaining {
        before.push(community_generation(&community, i).await);
    }
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: bea,
                reason: None,
            },
        )))
        .await;
    for (n, i) in remaining.into_iter().enumerate() {
        assert!(
            eventually(&community.sim, Duration::from_secs(180), || async {
                community_generation(&community, i).await > before[n]
            })
            .await,
            "node {i} receives the rotated key"
        );
    }
    // Long enough for every relayed copy of the notice to arrive and for
    // every cascade level to time out.
    tokio::time::sleep(Duration::from_secs(300)).await;
    community.sim.quiesce().await;
    for (n, i) in remaining.into_iter().enumerate() {
        assert_eq!(
            community_generation(&community, i).await,
            before[n] + 1,
            "node {i}: one kick moves the key one generation: {}",
            community.mek_list(i).await
        );
    }
}
