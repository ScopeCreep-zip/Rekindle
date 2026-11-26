//! Plan E3.2 S7d-keys.8: the unlock barrier holds our governance writes.
//! A held write is answered at once and signed in the background; seq
//! n + 1 is signed only after seq n's append landed, and a write made
//! while the barrier catches up is signed once a ring peer answered. A
//! leave made while catching up takes effect locally at once and survives
//! a restart; its `MemberLeft` is signed once the barrier passed.

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::unlock_barrier::BarrierPhase;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{
    ChannelMessageRequest, CommunityRequest, IpcRequest, LifecycleRequest,
};
use rekindle_netsim::{Fault, Writes};
use rekindle_types::display_catch_up::{PendingGovKind, PendingGovOp, PendingGovState};
use rekindle_types::governance::GovernanceEntry;

use crate::equivocation::{create_role, shows, unlock};
use crate::harness::{eventually, Community};
use crate::invite_unpublished::creator_record;

const SEEN: Duration = Duration::from_secs(180);

/// Node `i`'s own ops (its pseudonym's) by seq, as its op store holds them.
async fn own_seqs(community: &Community, i: usize) -> Vec<u64> {
    let me = community.pseudonym(i).await;
    let held = (community.nodes[i].host.governance())
        .governance_ops(&community.anchor)
        .await
        .unwrap_or_default();
    let mut seqs: Vec<u64> = held
        .into_iter()
        .filter(|op| hex::encode(op.author.0) == me)
        .map(|op| op.seq)
        .collect();
    seqs.sort_unstable();
    seqs
}

/// The seqs of node `i`'s own ops defining the role `name`.
async fn own_role_seqs(community: &Community, i: usize, name: &str) -> Vec<u64> {
    let me = community.pseudonym(i).await;
    let held = (community.nodes[i].host.governance())
        .governance_ops(&community.anchor)
        .await
        .unwrap_or_default();
    let mut seqs: Vec<u64> = (held.into_iter())
        .filter(|op| hex::encode(op.author.0) == me)
        .filter_map(|op| rekindle_codec::governance_op::decode_op(&op.encoded).ok())
        .filter(|op| {
            matches!(&op.op.body, GovernanceEntry::RoleDefinition { name: n, .. } if n == name)
        })
        .map(|op| op.op.seq)
        .collect();
    seqs.sort_unstable();
    seqs
}

/// The held role writes: an automatic `Checkpoint` (the anti-entropy
/// tick's) may be held beside them.
fn held_roles(held: &[PendingGovOp]) -> Vec<&PendingGovOp> {
    (held.iter())
        .filter(|op| op.kind == PendingGovKind::RoleEdited)
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn seq_n_plus_one_waits_for_seq_n_to_land() {
    let community = Community::of(3, 1717).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    // alpha (seq n) is signed and queued, its append cannot land: the
    // creator's op record refuses every set (a partition still reaches
    // the replicas beside node 0, where alpha lands).
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(creator_record(&community)),
    });
    create_role(&community, 0, "alpha").await;
    create_role(&community, 0, "bravo").await;
    tokio::time::sleep(SEEN).await;
    assert_eq!(own_role_seqs(&community, 0, "alpha").await.len(), 1);
    assert_eq!(
        own_role_seqs(&community, 0, "bravo").await,
        Vec::<u64>::new(),
        "bravo (seq n + 1) is not signed before alpha landed: {:?}",
        own_seqs(&community, 0).await
    );
    let barrier = community.nodes[0]
        .host
        .governance()
        .unlock_barrier(&community.anchor);
    let held = barrier.view().expect("bravo is listed").pending;
    assert_eq!(held_roles(&held).len(), 1, "only bravo is held: {held:?}");

    community.sim.fault(Fault::Heal);
    assert!(
        eventually(&community.sim, SEEN * 2, || async {
            shows(&community, 1, "alpha").await
                && shows(&community, 1, "bravo").await
                && shows(&community, 2, "bravo").await
        })
        .await,
        "both land once alpha's append landed"
    );
    let alpha = own_role_seqs(&community, 0, "alpha").await;
    let bravo = own_role_seqs(&community, 0, "bravo").await;
    assert_eq!(alpha.len(), 1, "{alpha:?}");
    assert_eq!(bravo, vec![alpha[0] + 1], "bravo is seq n + 1");
    assert!(
        eventually(&community.sim, SEEN, || async { barrier.view().is_none() }).await,
        "nothing stays held"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_write_held_while_catching_up_is_signed_once_a_ring_peer_answers() {
    let community = Community::of(3, 1818).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let before = own_seqs(&community, 0).await;

    community.nodes[0].ok(IpcRequest::Lock).await;
    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    unlock(&community, 0).await;
    // Answered at once although no ring peer answers the barrier.
    create_role(&community, 0, "delta").await;
    let barrier = community.nodes[0]
        .host
        .governance()
        .unlock_barrier(&community.anchor);
    tokio::time::sleep(SEEN).await;
    assert_eq!(barrier.phase(), BarrierPhase::CatchingUp);
    let view = barrier.view().expect("catching up");
    assert!(view.catching_up);
    let roles = held_roles(&view.pending);
    let [held] = roles.as_slice() else {
        panic!("delta is held: {view:?}");
    };
    assert_eq!(held.kind, PendingGovKind::RoleEdited);
    assert_eq!(held.state, PendingGovState::CatchingUp);
    assert_eq!(own_seqs(&community, 0).await, before, "nothing signed yet");
    assert!(!shows(&community, 1, "delta").await);

    community.sim.fault(Fault::Heal);
    assert!(
        eventually(&community.sim, SEEN * 2, || async {
            shows(&community, 1, "delta").await && shows(&community, 2, "delta").await
        })
        .await,
        "delta is signed once a ring peer answered"
    );
    assert_eq!(barrier.phase(), BarrierPhase::Passed);
    assert!(
        eventually(&community.sim, SEEN, || async { barrier.view().is_none() }).await,
        "the held list cleared"
    );
    assert_eq!(
        own_role_seqs(&community, 0, "delta").await.len(),
        1,
        "delta is signed once"
    );
}

/// Whether node `i` lists the community anywhere (its list or its views)
/// or still resolves its keys.
async fn still_holds(community: &Community, i: usize) -> bool {
    let node = &community.nodes[i];
    let listed = node
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityList,
        )))
        .await;
    let views = node
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        )))
        .await;
    let keys = node
        .call(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::MekList {
                community: community.anchor.clone(),
            },
        )))
        .await;
    listed.to_string().contains(&community.anchor)
        || views.to_string().contains(&community.anchor)
        || keys.is_ok()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_leave_while_catching_up_takes_effect_at_once_and_survives_a_restart() {
    let mut community = Community::of(3, 1919).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let p = community.pseudonym(2).await;
    assert!(
        still_holds(&community, 2).await,
        "a member holds the community"
    );

    community.nodes[2].ok(IpcRequest::Lock).await;
    community.sim.fault(Fault::Partition {
        a: vec![2],
        b: vec![0, 1],
    });
    unlock(&community, 2).await;
    community.nodes[2]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                anchor: community.anchor.clone(),
            },
        )))
        .await
        .expect("the leave is answered while catching up");
    assert!(
        !still_holds(&community, 2).await,
        "the leave takes effect locally at once"
    );

    let node = community.nodes.pop().expect("node 2");
    let node = node.restart(&community.sim, Duration::from_secs(1)).await;
    community.nodes.push(node);
    unlock(&community, 2).await;
    assert!(
        !still_holds(&community, 2).await,
        "a restart before the barrier passed does not restore the membership"
    );
    assert!(
        crate::devices::lists(&community, 0, &p, true).await,
        "nothing is signed while catching up"
    );

    community.sim.fault(Fault::Heal);
    assert!(
        crate::devices::lists(&community, 0, &p, false).await,
        "the held MemberLeft is signed once the barrier passed"
    );
    let gov = community.nodes[2].host.governance();
    assert!(
        eventually(&community.sim, SEEN, || async {
            gov.governance_state(&community.anchor).is_none()
        })
        .await,
        "then the leave completes: the community is forgotten"
    );
}

/// Plan E3.2 S7d-keys.3/.8: the ops our own signer made at create (the
/// genesis ops) and at join (our `MemberJoined`) are stored `signed_here`,
/// so a fresh creator's and a fresh joiner's barrier reads them as its own
/// signed head and passes, never as a restore, and their first write signs.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_fresh_creator_and_joiner_pass_the_barrier_and_their_first_write_signs() {
    let community = Community::of(3, 2020).await;
    // The joiner's first write is one a member without permissions makes:
    // taking a self-assignable role (the creator's write is a new role).
    let picked = crate::devices::self_assignable_role(&community, "picked").await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    for i in [0, 1] {
        community.nodes[i].ok(IpcRequest::Lock).await;
        unlock(&community, i).await;
        let barrier = community.nodes[i]
            .host
            .governance()
            .unlock_barrier(&community.anchor);
        assert!(
            eventually(&community.sim, SEEN, || async {
                barrier.phase() == BarrierPhase::Passed
            })
            .await,
            "node {i}: the barrier passes, never a restore: {:?}",
            barrier.phase()
        );
        let seen = if i == 0 {
            create_role(&community, 0, "first0").await;
            eventually(&community.sim, SEEN, || async {
                shows(&community, 2, "first0").await
            })
            .await
        } else {
            crate::devices::self_assign(&community, i, &picked)
                .await
                .expect("the joiner takes the role");
            let me = community.pseudonym(i).await;
            crate::devices::sees_role(&community, 2, &me, &picked).await
        };
        assert!(
            seen,
            "node {i}'s first write is signed: {:?}",
            own_seqs(&community, i).await
        );
    }
}

/// Plan E3.2 S7d-keys.8 on the desktop's mode: every unlock re-arms the
/// barrier. Node 1, a shell-mode member, passes it at its first unlock,
/// logs out and unlocks again in the same app session with every ring peer
/// silent: the barrier catches up again and its first write (a leave, an
/// op any member signs) is held, as at a first unlock, never signed on the
/// passed barrier the log-out left.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_re_unlock_in_one_app_session_re_arms_the_barrier_on_the_shell() {
    let community = crate::shell_member_route::with_shell_member(3, 1, 2121).await;
    let first = community.nodes[1]
        .host
        .governance()
        .unlock_barrier(&community.anchor);
    assert!(
        eventually(&community.sim, SEEN, || async {
            first.phase() == BarrierPhase::Passed
        })
        .await,
        "the first unlock's barrier passes"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    let p = community.pseudonym(1).await;

    community.nodes[1].ok(IpcRequest::Logout).await;
    community.sim.fault(Fault::Partition {
        a: vec![1],
        b: vec![0, 2],
    });
    unlock(&community, 1).await;
    let barrier = community.nodes[1]
        .host
        .governance()
        .unlock_barrier(&community.anchor);
    community.nodes[1]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                anchor: community.anchor.clone(),
            },
        )))
        .await
        .expect("the leave is answered while catching up");
    tokio::time::sleep(SEEN).await;
    assert_eq!(
        barrier.phase(),
        BarrierPhase::CatchingUp,
        "the re-unlock catches up again"
    );
    assert!(
        crate::devices::lists(&community, 0, &p, true).await,
        "nothing is signed while catching up"
    );

    community.sim.fault(Fault::Heal);
    assert!(
        eventually(&community.sim, SEEN * 2, || async {
            barrier.phase() == BarrierPhase::Passed
        })
        .await,
        "the barrier passes once a ring peer answered"
    );
    assert!(
        crate::devices::lists(&community, 0, &p, false).await,
        "the held MemberLeft is signed once the barrier passed"
    );
}

/// Plan E3.2 S7d-keys.8: an invite is never held. While the barrier
/// catches up, `InviteCreate` is refused at once with the catching-up
/// reason, before any network step and whatever else holds the
/// community's rebuild lock (a rebuild fetching missing deps); once the
/// barrier passed, the invite is created.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_invite_while_catching_up_is_refused_at_once_and_made_once_passed() {
    let community = Community::of(3, 1821).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.nodes[0].ok(IpcRequest::Lock).await;
    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    unlock(&community, 0).await;
    let governance = community.nodes[0].host.governance();
    let barrier = governance.unlock_barrier(&community.anchor);
    assert_eq!(barrier.phase(), BarrierPhase::CatchingUp);
    let invite = || {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteCreate {
                community: community.anchor.clone(),
                max_uses: 1,
                expires_seconds: None,
            },
        ))
    };
    let rebuild = governance.rebuild_lock(&community.anchor);
    let rebuilding = rebuild.lock().await;
    let answer = tokio::time::timeout(Duration::from_secs(5), community.nodes[0].call(invite()))
        .await
        .expect("the create is answered at once while catching up");
    let refused = match answer {
        Ok(created) => panic!("no invite while catching up: {created}"),
        Err(refused) => refused,
    };
    assert!(
        refused.message.contains("This community is catching up"),
        "{}",
        refused.message
    );
    drop(rebuilding);

    community.sim.fault(Fault::Heal);
    assert!(
        eventually(&community.sim, SEEN * 2, || async {
            barrier.phase() == BarrierPhase::Passed
        })
        .await,
        "the barrier passed once a ring peer answered"
    );
    let created = community.nodes[0].ok(invite()).await;
    assert!(
        created["url"].as_str().is_some_and(|url| !url.is_empty()),
        "{created}"
    );
}
