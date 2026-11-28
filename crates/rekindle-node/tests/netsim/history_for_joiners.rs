//! Plan D16 (ADR 0012 "History for joiners", MSC4268): the inviter
//! delivers every community key generation the joiner's role may read, so
//! a post sealed under a generation a departure rotation retired before
//! the join still opens for the joiner. Before D16's bundle the inviter
//! sent only the current key, and the joiner's catch-up passed the older
//! entry as "of no channel we read" (`channel/outbox_hold.rs`).

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};

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
async fn a_joiner_reads_a_post_sealed_under_a_generation_rotated_before_it_joined() {
    let mut community = Community::create(4, 557).await;
    community.join(1).await;
    community.join(2).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let before = community_generation(&community, 0).await;
    community
        .post(1, "sealed under the first key")
        .await
        .expect("the member posts");
    assert!(
        community
            .reads(0, "sealed under the first key", Duration::from_secs(120))
            .await,
        "the creator reads the post"
    );
    let removed = community.pseudonym(2).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: removed,
                reason: None,
            },
        )))
        .await;
    for i in [0, 1] {
        assert!(
            eventually(&community.sim, Duration::from_secs(180), || async {
                community_generation(&community, i).await > before
            })
            .await,
            "node {i} takes the departure rotation"
        );
    }
    community.join(3).await;
    assert!(
        community
            .history_holds(3, "sealed under the first key", Duration::from_secs(300))
            .await,
        "the joiner opens the post sealed under the retired generation: {}",
        community.mek_list(3).await
    );
}
