//! Live walk 23 finding M87 (2026-11-18): a kicked member re-joins with a
//! fresh invite under the same community pseudonym. Its `MemberJoined` is
//! the next op of its own chain after its first `MemberJoined`, as a Matrix
//! re-join is a new `m.room.member` join in the same user's history and a
//! Signal group re-add keeps the member's identity: the kicked member's
//! read-only history row becomes its membership again, every member lists
//! it, and its next post is read.
//!
//! As live, the member's first join had its bootstrap exchange time out
//! (the inviter answered no call) and its `MemberJoined` never landed on
//! its op log (queued there; the inviter merged it from the mesh), so the
//! walk from the inviter's frontier cannot fetch it, nor the removal citing
//! it: the re-join's chain continues from the op it holds itself, kept with
//! the history. Without that it restarted at seq 0 and both its joins were
//! dropped as an equivocation.

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest, MemberRequest};
use rekindle_netsim::{Fault, Records, Writes};
use rekindle_types::dht_layout::member_record;
use rekindle_types::id::PseudonymKey;

use crate::harness::{eventually, Community};
use crate::moderation::listed;

const LIMIT: Duration = Duration::from_secs(180);

/// Whether nodes `nodes` all list `who` as a member within [`LIMIT`].
async fn all_list(community: &Community, who: &str, nodes: &[usize]) -> bool {
    eventually(&community.sim, LIMIT, || async {
        let mut every = true;
        for &i in nodes {
            every &= listed(community, i, who).await.is_some();
        }
        every
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kicked_member_rejoins_and_posts() {
    let mut community = Community::create(2, 331).await;

    // Bea's first join: the inviter answers no call, so the bootstrap
    // exchange times out, and its op log refuses the `MemberJoined`.
    let link = community.invite().await;
    community.sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Subkey {
            subkey: member_record::GOV_HEAD,
            record: Some(Records::CreatedDflt(member_record::SUBKEY_COUNT)),
        },
    });
    community.sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: true,
    });
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: link },
        )))
        .await;
    community.sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: false,
    });
    let bea = community.pseudonym(1).await;
    assert!(
        all_list(&community, &bea, &[0, 1]).await,
        "the inviter lists bea, merged from the mesh"
    );
    // That op log, only, stays refusing: the re-join's fresh member record
    // takes its writes.
    let first_record = (community.nodes[0].host.governance())
        .governance_state(&community.governance_key)
        .and_then(|state| {
            (state.members.get(&PseudonymKey::from_hex_lossy(&bea)))
                .map(|m| m.member_record.clone())
        })
        .expect("bea's member record");
    community.sim.fault(Fault::Heal);
    community.sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Record(first_record),
    });

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
                reason: None,
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, LIMIT, || async {
            community.info(1).await["removed"].is_object()
        })
        .await,
        "bea merges its own removal: {}",
        community.info(1).await
    );
    community.sim.quiesce().await;

    // A fresh invite; `join_with` asserts the re-joiner holds the key.
    community.join(1).await;
    assert_eq!(
        community.pseudonym(1).await,
        bea,
        "the same community pseudonym"
    );
    assert!(
        community.info(1).await["removed"].is_null(),
        "the re-join clears the removal: {}",
        community.info(1).await
    );
    assert!(
        all_list(&community, &bea, &[0, 1]).await,
        "the inviter and the re-joiner list bea again"
    );
    community
        .post(1, "back after the kick")
        .await
        .expect("the re-joiner posts");
    assert!(
        community.reads(0, "back after the kick", LIMIT).await,
        "the inviter reads the re-joiner's post"
    );
}
