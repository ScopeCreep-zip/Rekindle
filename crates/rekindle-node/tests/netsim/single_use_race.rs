//! Two joiners redeem one single-use invite at once (finding I1): the merge
//! counts exactly one of them (`merge/gates.rs`, the invite's `max_uses`),
//! and every member's list shows exactly the members the merge admitted. A
//! joiner the merge refused announces its join on gossip and writes its
//! own record all the same; a member list that showed it would show a
//! non-member as one (ADR 0011: membership is the merged state's alone).
//! When the inviter has already stored the other join, its bootstrap
//! answer refuses the loser at once (`bootstrap.rs` `admitted_deps`): the
//! loser is told, withdraws, and holds no community.

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::{Fault, Writes};

use crate::harness::{eventually, Community, Refused};

/// Node `i`'s merged member count.
async fn member_count(community: &Community, i: usize) -> u64 {
    community.info(i).await["member_count"]
        .as_u64()
        .unwrap_or_default()
}

/// The pseudonyms on node `i`'s member list, the list every frontend reads.
async fn listed(community: &Community, i: usize) -> BTreeSet<String> {
    let members = community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                anchor: community.anchor.clone(),
            },
        )))
        .await;
    members
        .as_array()
        .expect("a member list")
        .iter()
        .map(|m| m["pseudonymKey"].as_str().expect("a pseudonym").to_owned())
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_single_use_invite_lists_one_of_two_racing_joiners() {
    let mut community = Community::create(4, 4411).await;
    community.join(1).await;
    let link = community.invite().await;
    let join = |i: usize| {
        community.nodes[i].call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin {
                invite: link.clone(),
            },
        )))
    };
    // Both redeem the one use before either join is merged anywhere.
    let (a, b) = tokio::join!(join(2), join(3));
    assert!(
        a.is_ok() || b.is_ok(),
        "at least one racing join completes: {a:?} {b:?}"
    );
    let mut admitted = BTreeSet::new();
    for i in 0..2 {
        admitted.insert(community.pseudonym(i).await);
    }
    // A joiner the inviter refused was told so and holds no community; a
    // joiner told it joined has its pseudonym on the merged lists or not.
    let mut joined = Vec::new();
    for (i, outcome) in [(2, &a), (3, &b)] {
        match outcome {
            Ok(_) => joined.push(community.pseudonym(i).await),
            Err(Refused { code, message, .. }) => {
                assert!(
                    *code == 409 && message == "This invite has already been used.",
                    "node {i}'s join is refused by the inviter, not lost: {message}"
                );
                assert!(
                    eventually(&community.sim, Duration::from_secs(60), || async {
                        community.nodes[i]
                            .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                                LifecycleRequest::CommunityInfo {
                                    anchor: community.anchor.clone(),
                                },
                            )))
                            .await
                            .is_err()
                    })
                    .await,
                    "node {i}'s refused join is withdrawn"
                );
            }
        }
    }
    // Every member merges both joins and counts exactly one of them.
    let settled = eventually(&community.sim, Duration::from_secs(600), || async {
        member_count(&community, 0).await == 3 && member_count(&community, 1).await == 3
    })
    .await;
    assert!(settled, "both members count the one admitted joiner");
    // Gossip, records and beats of the refused joiner keep arriving; the
    // lists must still hold only the admitted members.
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    for i in 0..2 {
        assert_eq!(
            member_count(&community, i).await,
            3,
            "node {i} counts one joiner"
        );
        let shown = listed(&community, i).await;
        assert!(
            admitted.is_subset(&shown),
            "node {i} lists both members: {shown:?}"
        );
        assert_eq!(
            shown.len(),
            3,
            "node {i} lists exactly the three admitted members: {shown:?}"
        );
        let racing: Vec<_> = joined.iter().filter(|p| shown.contains(*p)).collect();
        assert_eq!(
            racing.len(),
            1,
            "node {i} lists exactly one racing joiner, one told it joined: {shown:?}"
        );
    }
    assert_eq!(
        listed(&community, 0).await,
        listed(&community, 1).await,
        "every member lists the same admitted joiner"
    );
}

/// The inviter refuses the second redemption of a single-use invite with
/// its typed reason (plan E3.2 join step 5; `bootstrap.rs` `admitted_deps`
/// → `merge/gates.rs` `invite_gate`: the one use is taken →
/// `InviteUsedUp`; `daemon/handler.rs` answers `bootstrapRefused`). Every
/// write of the inviter's member record lands below consensus
/// (`Fault::WriteFails`, as a Veilid `set_value` can) from before the first
/// join, so the frontier the second joiner walks is the one that published
/// the invite: its own check (`join_flow.rs` `check_invite`) finds the
/// invite live, and only the inviter, which merged the first join, can
/// refuse it. The joiner is told 409 "This invite has already been used.",
/// withdraws, holds no community, and no member counts it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_used_up_single_use_invite_is_refused_with_its_reason() {
    let mut community = Community::create(3, 4412).await;
    let link = community.invite().await;
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(crate::invite_unpublished::creator_record(&community)),
    });
    community.join_with(1, link.clone()).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(600), || async {
            member_count(&community, 0).await == 2 && member_count(&community, 1).await == 2
        })
        .await,
        "both members merged the first join"
    );
    // The second joiner walks the inviter's frontier as the DHT holds it.
    let second = community.nodes[2]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: link },
        )))
        .await;
    match &second {
        Err(Refused { code, message, .. }) => assert!(
            *code == 409 && message == "This invite has already been used.",
            "the used-up invite is refused for its reason: {code} {message}"
        ),
        Ok(answer) => panic!("a second redemption of a single-use invite joined: {answer}"),
    }
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            community.nodes[2]
                .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityInfo {
                        anchor: community.anchor.clone(),
                    },
                )))
                .await
                .is_err()
        })
        .await,
        "the refused join is withdrawn"
    );
    community.sim.fault(Fault::Heal);
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    for i in 0..2 {
        assert_eq!(
            member_count(&community, i).await,
            2,
            "node {i} counts no refused joiner"
        );
    }
}
