//! Live A/B finding (2026-11-17): a member that left re-joins with a fresh
//! invite under the same community pseudonym (its `MemberJoined` is a later
//! op of its own chain, following its `MemberLeft`). The inviter merges the
//! re-join through the bootstrap exchange, every member lists it again, it
//! receives the community key (D16) and every member reads its next post,
//! a member joining later too.

use std::time::Duration;

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

/// The pseudonyms node `i` lists as the community's members.
async fn members(community: &Community, i: usize) -> Vec<String> {
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r["pseudonymKey"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

async fn all_list(community: &Community, who: &str, listed: bool, nodes: &[usize]) -> bool {
    eventually(&community.sim, LIMIT, || async {
        let mut every = true;
        for &i in nodes {
            every &= members(community, i).await.iter().any(|p| p == who) == listed;
        }
        every
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_that_left_rejoins_and_posts() {
    let mut community = Community::create(4, 317).await;
    community.join(1).await;
    community.join(2).await;
    let bea = community.pseudonym(1).await;
    assert!(
        all_list(&community, &bea, true, &[0, 2]).await,
        "every member lists bea"
    );
    community
        .post(1, "before leaving")
        .await
        .expect("bea posts");
    assert!(community.reads(2, "before leaving", LIMIT).await);

    community.nodes[1]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .expect("the leave commits");
    assert!(
        all_list(&community, &bea, false, &[0, 2]).await,
        "bea is gone from every list"
    );

    // A fresh invite; `join_with` asserts the re-joiner holds the key.
    community.join(1).await;
    assert_eq!(
        community.pseudonym(1).await,
        bea,
        "the same community pseudonym"
    );
    assert!(
        all_list(&community, &bea, true, &[0, 2]).await,
        "every member lists bea again"
    );
    community
        .post(1, "back again")
        .await
        .expect("the re-joiner posts");
    for i in [0, 2] {
        assert!(
            community.reads(i, "back again", LIMIT).await,
            "node {i} reads the re-joiner's post"
        );
    }
    // A member joining after the re-join walks the op DAG by `OpRef`
    // through the re-joiner's chain, whose `MemberJoined` lives on its
    // fresh member record.
    community.join(3).await;
    assert!(
        all_list(&community, &bea, true, &[3]).await,
        "a later joiner lists the re-joiner"
    );
    assert!(
        community.reads(3, "back again", LIMIT).await,
        "a later joiner reads the re-joiner's post"
    );
}
