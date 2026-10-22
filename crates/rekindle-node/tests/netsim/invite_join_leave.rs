//! Plan E3.1 D1b: a community is created, a member joins through an
//! invite, both list each other, and the member leaves: it is gone from the
//! creator's member list and the leaver holds nothing of the community.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;

use crate::harness::{eventually, Community};

/// The pseudonyms node `i` lists as the community's members.
async fn members(community: &Community, i: usize) -> Vec<String> {
    community.nodes[i]
        .call(IpcRequest::CommunityMembers {
            governance_key: community.governance_key.clone(),
        })
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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn invite_join_leave() {
    let community = Community::of(2, 83).await;
    let ann = community.pseudonym(0).await;
    let bea = community.pseudonym(1).await;
    for (i, other) in [(0, &bea), (1, &ann)] {
        assert!(
            eventually(&community.sim, Duration::from_secs(120), || async {
                members(&community, i).await.contains(other)
            })
            .await,
            "node {i} lists the other member"
        );
    }

    community.nodes[1]
        .call_held(IpcRequest::CommunityLeave {
            governance_key: community.governance_key.clone(),
        })
        .await
        .expect("the leave commits");

    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            !members(&community, 0).await.contains(&bea)
        })
        .await,
        "the leaver is gone from the creator's member list"
    );
    let listed = community.nodes[1].ok(IpcRequest::CommunityList).await;
    assert!(
        !listed.to_string().contains(&community.governance_key),
        "the leaver no longer lists the community: {listed}"
    );
    let views = community.nodes[1].ok(IpcRequest::CommunityViews).await;
    assert_eq!(views.as_array().map(Vec::len), Some(0), "{views}");
    let refused = community.nodes[1]
        .call(IpcRequest::CommunityMembers {
            governance_key: community.governance_key.clone(),
        })
        .await
        .expect_err("the leaver's records of the community are gone");
    assert_eq!(refused.code, 404, "{}", refused.message);
}
