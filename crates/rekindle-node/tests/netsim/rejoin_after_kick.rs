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
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, PASSPHRASE};
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

/// Bea's pseudonym and her first member record, which never held her
/// first `MemberJoined`.
struct Rejoined {
    bea: String,
    first_record: String,
}

/// Node 1 (bea) joins with its first `MemberJoined` never landing on its op
/// log, is kicked by node 0, and re-joins through a fresh invite; returns
/// bea's pseudonym. Bea's first member record stays refusing writes. A
/// node in `apart` is partitioned from nodes 0 and 1 from before bea's
/// first join until after her re-join (the caller heals it).
async fn kick_and_rejoin(community: &mut Community, apart: &[usize]) -> Rejoined {
    let partition = || Fault::Partition {
        a: apart.to_vec(),
        b: vec![0, 1],
    };
    community.sim.fault(partition());
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
        all_list(community, &bea, &[0, 1]).await,
        "the inviter lists bea, merged from the mesh"
    );
    // That op log, only, stays refusing: the re-join's fresh member record
    // takes its writes.
    let first_record = (community.nodes[0].host.governance())
        .governance_state(&community.anchor)
        .and_then(|state| {
            (state.members.get(&PseudonymKey::from_hex_lossy(&bea)))
                .map(|m| m.member_record.clone())
        })
        .expect("bea's member record");
    community.sim.fault(Fault::Heal);
    community.sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Record(first_record.clone()),
    });
    community.sim.fault(partition());

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
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
        all_list(community, &bea, &[0, 1]).await,
        "the inviter and the re-joiner list bea again"
    );
    Rejoined { bea, first_record }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kicked_member_rejoins_and_posts() {
    let mut community = Community::create(2, 331).await;
    kick_and_rejoin(&mut community, &[]).await;
    community
        .post(1, "back after the kick")
        .await
        .expect("the re-joiner posts");
    assert!(
        community.reads(0, "back after the kick", LIMIT).await,
        "the inviter reads the re-joiner's post"
    );
}

/// Node `i`'s merged members and removals, by pseudonym.
fn roster(community: &Community, i: usize) -> (Vec<PseudonymKey>, Vec<PseudonymKey>) {
    let state = (community.nodes[i].host.governance())
        .governance_state(&community.anchor)
        .expect("a merged governance state");
    let mut members: Vec<PseudonymKey> = state.members.keys().cloned().collect();
    let mut removed: Vec<PseudonymKey> = state.removed.keys().cloned().collect();
    members.sort_by_key(|k| k.0);
    removed.sort_by_key(|k| k.0);
    (members, removed)
}

/// A third member joins after the kick-and-rejoin, through a fresh invite
/// from the owner: its walk reaches bea's chain, whose seq 0 the owner's
/// ops cite by bea's first member record, which never held it. It merges
/// the owner's roster (bea listed and not departed, the kick still
/// recorded) and reads bea's next post, as a Matrix server joining later
/// replays the leave and the re-join in order and ends with the member
/// joined.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_joining_after_a_kick_and_rejoin_sees_the_rejoiner() {
    let mut community = Community::create(3, 337).await;
    let bea = kick_and_rejoin(&mut community, &[]).await.bea;
    let owner = community.pseudonym(0).await;

    community.join(2).await;
    assert!(
        all_list(&community, &bea, &[0, 1, 2]).await,
        "the third member lists bea"
    );
    assert!(
        listed(&community, 2, &owner).await.is_some(),
        "the third member lists the owner"
    );
    let bea_key = PseudonymKey::from_hex_lossy(&bea);
    let merged = eventually(&community.sim, LIMIT, || async {
        roster(&community, 2) == roster(&community, 0)
    })
    .await;
    let (members, removed) = roster(&community, 2);
    assert!(
        merged,
        "the third member merges the owner's roster: {:?} vs {:?}",
        roster(&community, 2),
        roster(&community, 0)
    );
    assert!(members.contains(&bea_key), "bea is a member");
    // `removed` keeps the kick's stamp, which a re-join must come after
    // (`GovernanceState::removed`); a re-joined member is not departed.
    assert!(removed.contains(&bea_key), "the kick stays recorded");
    let departed = (community.nodes[2].host.governance())
        .governance_state(&community.anchor)
        .is_some_and(|state| state.is_departed(&bea_key));
    assert!(!departed, "bea is not departed for the third member");

    community
        .post(1, "seen by the third member")
        .await
        .expect("the re-joiner posts");
    assert!(
        community.reads(2, "seen by the third member", LIMIT).await,
        "the third member reads the re-joiner's post"
    );
    assert!(
        community.reads(0, "seen by the third member", LIMIT).await,
        "the owner reads the re-joiner's post"
    );
}

/// Nodes `nodes` lock, then unlock with the network up; each one's login
/// network stage rebuilds the community from its stored ops
/// (`rebuild_ops`, `GovernanceRebuilt`), fetching only the deps it lacks.
async fn restart(community: &Community, nodes: &[usize]) {
    for &i in nodes {
        community.nodes[i].ok(IpcRequest::Lock).await;
    }
    for &i in nodes {
        let node = &community.nodes[i];
        let identity = node.public_key.clone().expect("an identity");
        node.ok(IpcRequest::Unlock {
            identity,
            passphrase: PASSPHRASE.into(),
        })
        .await;
        let events = node.events().await;
        let rebuilt = eventually(&community.sim, LIMIT, || async {
            events.any(|e| {
                matches!(
                    e,
                    SubscriptionEvent::Governance(GovernanceEvent::GovernanceRebuilt { community: c })
                        if *c == community.anchor
                )
            })
        })
        .await;
        assert!(rebuilt, "node {i} rebuilds the community after its restart");
    }
}

/// After a kick-and-rejoin, the owner and a third member (each holding
/// bea's re-join op, the third from its join walk) restart: the rebuild
/// re-verifies their stored ops, where bea's seq 0 is cited at her first
/// member record, which never held it (`op_walk::locators` on the join
/// walk). Each still lists bea, not departed, and reads her next post,
/// as a Matrix server restarting replays its stored room state, the leave
/// and the re-join in order.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_restart_after_a_kick_and_rejoin_keeps_the_rejoiner() {
    restart_after_rejoin().await;
}

/// One seed, one trace (ADR 0021, C9.3) through both liveness tasks of a
/// node (`run_publisher` and `run_self_tests`): under one shared tokio
/// watch they woke in an order tokio draws from the runtime's own RNG
/// (`BigNotify::notified`, `thread_rng_n(8)`), so they took their liveness
/// draws in either order and seed 347 gave two traces, one failing (RK1).
/// Each run is a runtime of its own, so each draws another order.
#[test]
fn a_restart_after_a_kick_and_rejoin_has_one_trace_per_seed() {
    fn run() -> [u8; 32] {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("a runtime")
            .block_on(restart_after_rejoin())
    }
    let first = run();
    // Six runtimes, as the retired stretch's guard (`group_dm.rs`).
    for _ in 0..5 {
        assert_eq!(run(), first, "one seed, one trace");
    }
}

/// [`a_restart_after_a_kick_and_rejoin_keeps_the_rejoiner`]'s run under seed
/// 347; returns its trace hash.
async fn restart_after_rejoin() -> [u8; 32] {
    let mut community = Community::create(3, 347).await;
    let bea = kick_and_rejoin(&mut community, &[]).await.bea;
    community.join(2).await;
    assert!(
        all_list(&community, &bea, &[0, 1, 2]).await,
        "every member lists bea before the restart"
    );
    community.sim.quiesce().await;

    restart(&community, &[0, 2]).await;
    let bea_key = PseudonymKey::from_hex_lossy(&bea);
    for i in [0, 2] {
        assert!(
            listed(&community, i, &bea).await.is_some(),
            "node {i} lists bea after its restart"
        );
        let state = (community.nodes[i].host.governance())
            .governance_state(&community.anchor)
            .expect("a rebuilt governance state");
        assert!(
            state.members.contains_key(&bea_key) && !state.is_departed(&bea_key),
            "node {i}: bea is a member, not departed, after its restart"
        );
    }

    community
        .post(1, "after their restart")
        .await
        .expect("the re-joiner posts");
    for i in [0, 2] {
        assert!(
            community.reads(i, "after their restart", LIMIT).await,
            "node {i} reads the re-joiner's post after its restart"
        );
    }
    community.sim.trace().hash()
}

/// A member that joined before bea and was cut off from her first join
/// until after her re-join never held bea's seq 0: her first record never
/// held it, and the gossip of it, of the kick and of the re-join all went
/// by while it was partitioned. Once healed, the ops it then receives
/// (the re-join `MemberJoined`, the kick) cite seq 0 at bea's first record,
/// and its rebuild (`rebuild_ops`) must find it where the re-join mirrored
/// it, at her fresh record (ADR 0011: an `OpRef` is a locator). It lists
/// bea, not departed, and reads her next post, as a Matrix server back
/// from a netsplit fetches the missing events of a re-joined member from
/// any server in the room (`/get_missing_events`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_cut_off_during_a_kick_and_rejoin_sees_the_rejoiner() {
    let mut community = Community::create(3, 353).await;
    community.join(2).await;
    let rejoined = kick_and_rejoin(&mut community, &[2]).await;
    let bea = rejoined.bea;
    // The partition ends; bea's first record keeps refusing writes, put
    // back before any timer runs, so seq 0 never lands there.
    community.sim.fault(Fault::Heal);
    community.sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Record(rejoined.first_record),
    });

    assert!(
        all_list(&community, &bea, &[0, 1, 2]).await,
        "the member cut off lists bea once healed"
    );
    let bea_key = PseudonymKey::from_hex_lossy(&bea);
    let merged = eventually(&community.sim, LIMIT, || async {
        roster(&community, 2) == roster(&community, 0)
    })
    .await;
    assert!(
        merged,
        "the member cut off merges the owner's roster: {:?} vs {:?}",
        roster(&community, 2),
        roster(&community, 0)
    );
    let departed = (community.nodes[2].host.governance())
        .governance_state(&community.anchor)
        .is_some_and(|state| state.is_departed(&bea_key));
    assert!(!departed, "bea is not departed for the member cut off");

    community
        .post(1, "seen after the netsplit")
        .await
        .expect("the re-joiner posts");
    assert!(
        community.reads(2, "seen after the netsplit", LIMIT).await,
        "the member cut off reads the re-joiner's post"
    );
}
