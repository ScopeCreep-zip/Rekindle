//! A join through an invite minted right after the community was created
//! (live walk B60 → B61, finding IV1: on Linux 1 in 4 such joins was refused
//! 404 "This invite is not one the community knows"). The inviter hands the
//! link out only once its `InviteCreated` and its frontier are on `M`/2 and
//! `M`/3 (`apply::write_invite_created`, plan E3.2 S6 (4)), so a joiner that
//! walks from that frontier at once, with no settle wait, finds the invite
//! (`join_flow::check_invite`) on every seed.
//!
//! The seed set is `0..IV1_SEEDS` (default 8); a sweep sets it higher. Every
//! refused seed is collected with its trace hash into the assertion's message,
//! so one run names them all.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};

use rekindle_netsim::Fault;

use crate::harness::Community;

fn seeds() -> u64 {
    std::env::var("IV1_SEEDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(8)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_fresh_invite_is_known_to_an_immediate_join() {
    let mut refused = Vec::new();
    for seed in 0..seeds() {
        let mut community = Community::create(2, 9_100 + seed).await;
        let link = community.invite().await;
        let joined = community.nodes[1]
            .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityJoin { invite: link },
            )))
            .await;
        if let Err(refusal) = joined {
            let hash = hex::encode(community.sim.trace().hash());
            refused.push((9_100 + seed, refusal.code, refusal.message, hash));
        }
    }
    assert!(
        refused.is_empty(),
        "joins refused through a fresh invite: {refused:?}"
    );
}

/// Decision (28)'s stale read (plan IV1b): the inviter and the joiner sit on
/// two sides of a placement split (`Fault::Partition`, node 2 reached by both),
/// so the inviter's `InviteCreated` and frontier land on the replicas beside
/// its side and node 2's, while the joiner's forced frontier read (0.5.8
/// `get_value.rs` `check_done`: done at `get_value_count` holders of the
/// newest value it saw) can settle on the pre-invite frontier its closest
/// replicas still agree on. The link names the op and the frontier seq it
/// was minted at, so the join reads past the stale frontier; no seed may be
/// refused `InviteUnknown`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stale_frontier_read_does_not_refuse_a_fresh_invite() {
    let mut refused = Vec::new();
    for seed in 0..seeds() {
        let seed = 9_300 + seed;
        let mut community = Community::create(5, seed).await;
        community.sim.fault(Fault::Partition {
            a: vec![0],
            b: vec![1, 3, 4],
        });
        let link = community.invite().await;
        let joined = community.nodes[1]
            .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityJoin { invite: link },
            )))
            .await;
        if let Err(refusal) = joined {
            let hash = hex::encode(community.sim.trace().hash());
            refused.push((seed, refusal.code, refusal.message, hash));
        }
    }
    assert!(
        refused.is_empty(),
        "joins refused across a placement split: {refused:?}"
    );
}
