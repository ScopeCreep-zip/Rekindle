//! Plan E3.2 S7s.4 (SS4): a creator offers the community to a member, who
//! accepts by creating the successor (its cert names this community, its
//! creator set the links' new pseudonyms) and publishing the acceptance
//! citing the offer; every member is then offered the successor and
//! follows it. A withdrawn offer cannot be accepted and a later offer
//! supersedes an earlier one (lead decisions (24)–(26)).

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};

use crate::harness::{eventually, Community};

fn lifecycle(request: LifecycleRequest) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(request))
}

fn offer(community: &Community, to: String) -> IpcRequest {
    lifecycle(LifecycleRequest::CommunityOfferTransfer {
        anchor: community.anchor.clone(),
        transfer_to: to,
    })
}

fn accept(community: &Community) -> IpcRequest {
    lifecycle(LifecycleRequest::CommunityAcceptTransfer {
        anchor: community.anchor.clone(),
        co_creators: Vec::new(),
    })
}

/// Waits until every node's merged state holds `rows` successor ops.
async fn rows_everywhere(community: &Community, rows: usize) {
    let all = eventually(&community.sim, Duration::from_secs(180), || async {
        community.nodes.iter().all(|node| {
            (node.host.governance().governance_state(&community.anchor))
                .is_some_and(|state| state.successors.len() >= rows)
        })
    })
    .await;
    assert!(all, "every member merged {rows} successor ops");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_transfer_completes_and_members_follow_the_successor() {
    let community = Community::of(3, 466).await;
    let recipient = community.pseudonym(1).await;
    community.nodes[0].ok(offer(&community, recipient)).await;
    rows_everywhere(&community, 1).await;
    let created = community.nodes[1].ok(accept(&community)).await;
    let new_anchor = created["anchor"]
        .as_str()
        .expect("the successor")
        .to_owned();
    let offered = eventually(&community.sim, Duration::from_secs(300), || async {
        let request = lifecycle(LifecycleRequest::CommunitySuccessors {
            anchor: community.anchor.clone(),
        });
        (community.nodes[2].call(request).await.ok())
            .and_then(|v| v.as_array().cloned())
            .is_some_and(|offers| {
                offers
                    .iter()
                    .any(|o| o["newAnchor"] == new_anchor.as_str() && o["reason"] == "transfer")
            })
    })
    .await;
    assert!(offered, "the member is offered the accepted successor");
    community.nodes[2]
        .ok(lifecycle(LifecycleRequest::CommunityFollowSuccessor {
            anchor: community.anchor.clone(),
            new_anchor: new_anchor.clone(),
        }))
        .await;
    let joined = eventually(&community.sim, Duration::from_secs(120), || async {
        (community.nodes[2]
            .call(lifecycle(LifecycleRequest::CommunityList))
            .await)
            .is_ok_and(|v| v.to_string().contains(&new_anchor))
    })
    .await;
    assert!(joined, "the member follows the successor");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_withdrawn_offer_cannot_be_accepted() {
    let community = Community::of(2, 467).await;
    let recipient = community.pseudonym(1).await;
    community.nodes[0].ok(offer(&community, recipient)).await;
    community.nodes[0]
        .ok(lifecycle(LifecycleRequest::CommunityWithdrawTransfer {
            anchor: community.anchor.clone(),
        }))
        .await;
    rows_everywhere(&community, 2).await;
    let refused = community.nodes[1].call(accept(&community)).await;
    let why = format!("{:?}", refused.expect_err("a withdrawn offer"));
    assert!(why.contains("no standing offer"), "refused: {why}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_later_offer_supersedes_an_earlier_one() {
    let community = Community::of(3, 468).await;
    let (first, second) = (community.pseudonym(1).await, community.pseudonym(2).await);
    community.nodes[0].ok(offer(&community, first)).await;
    community.nodes[0].ok(offer(&community, second)).await;
    rows_everywhere(&community, 2).await;
    let refused = community.nodes[1].call(accept(&community)).await;
    assert!(refused.is_err(), "the superseded recipient cannot accept");
    community.nodes[2].ok(accept(&community)).await;
}
