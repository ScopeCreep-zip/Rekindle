//! A join beside a checkpoint's cut, by construction (plan E3.2 S7c.8 (3)):
//! the creator (node 0, the only height-1 proposer) and a member inviter
//! (node 1) are partitioned while the creator writes an op the inviter
//! never sees and then proposes the height-1 checkpoint over it. A joiner
//! (node 2) joins through the inviter's invite on the inviter's side: its
//! inviter holds no hint, so the `MemberJoined` cites the genesis base and
//! the inviter's latest op (`join_deps.rs`), never the creator's newest op,
//! a cut head. After the heal the creator merges that join and invites a
//! later joiner (node 3); the invite op cites the creator's frontier, so
//! the join beside the cut is in the walk above the cut from the
//! creator's published frontier (`M`/3, `join_flow.rs` `read_frontier`).
//! The later joiner resumes from the creator's hint over a `rest` holding
//! that join: a non-power op whose past holds every applied cut power op
//! (the genesis ops), so the snapshot answers for it and the resume
//! stands. With every op beside the cut refused, the hint is lost and the
//! later joiner walks in full (checked by hand, 2026-11-18).

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, Community};

/// Node `i`'s merged member count.
async fn members(community: &Community, i: usize) -> u64 {
    community.info(i).await["member_count"]
        .as_u64()
        .unwrap_or_default()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_beside_the_cut_keeps_the_hint_for_later_joiners() {
    let mut community = Community::create(4, 2314).await;
    community.join(1).await;
    // The inviter's own invite, merged by the creator before the cut.
    let beside_link = community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteCreate {
                community: community.anchor.clone(),
                max_uses: 1,
                expires_seconds: None,
            },
        )))
        .await["url"]
        .as_str()
        .expect("an invite link")
        .to_owned();
    tokio::time::sleep(Duration::from_secs(5)).await;
    community.sim.fault(Fault::Partition {
        a: vec![0, 3],
        b: vec![1, 2],
    });
    // The creator's newest op, a cut head the inviter's side never sees.
    community.invite().await;
    community.join_with(2, beside_link).await;
    // The creator proposes height 1 over its newest op after a quiet sweep.
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    assert_eq!(
        members(&community, 0).await,
        2,
        "the creator cut its checkpoint without the join"
    );
    community.sim.fault(Fault::Heal);
    let merged = eventually(&community.sim, Duration::from_secs(300), || async {
        members(&community, 0).await == 3
    })
    .await;
    assert!(merged, "the creator merged the join beside its cut");
    // The later joiner's invite cites the creator's frontier, the join
    // beside the cut among it, so the joiner's walk above the cut (from
    // the inviter's published frontier) reaches that join.
    let later_link = community.invite().await;
    community.join_with(3, later_link).await;
    for early in 1..=2 {
        let resumed = crate::logs::logged(early, &["join resumed from a checkpoint"]);
        assert!(
            resumed.is_empty(),
            "node {early} joined with no hint: {resumed:?}"
        );
    }
    let resumed = crate::logs::logged(3, &["join resumed from a checkpoint"]);
    assert_eq!(
        resumed.len(),
        1,
        "the later join resumed from the hint over the join beside the cut: {resumed:?}"
    );
    assert!(
        resumed[0].contains("height=1"),
        "from height 1: {resumed:?}"
    );
    assert!(crate::logs::warned("roster checksum").is_empty());
}
