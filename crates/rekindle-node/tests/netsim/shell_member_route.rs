//! Plan E3.4 S5 on the shell mode (Q3, S4 premise (5)): a desktop-mode
//! member (its Host on an adopted transport, `HostMode::Shell`) and a
//! daemon member find each other by the routes their `M`/6 published, a
//! key rotation and a `RequestMEK` answer reach the shell member by its
//! `M`/6 route alone, and the runtime's `online_members` is one source on
//! both modes (the roster ∩
//! present beats ∩ the `M`/6 cache), never the daemon-only registry
//! overlay that left a desktop with nobody online.

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, start_with, Community};
use crate::shell_node::SimMode;

/// A community of `n` whose node `shell` is a desktop-mode member; every
/// other node is a daemon, node 0 the creator. All have joined.
pub(crate) async fn with_shell_member(n: usize, shell: usize, seed: u64) -> Community {
    let mode = |i| {
        if i == shell {
            SimMode::Shell
        } else {
            SimMode::Daemon
        }
    };
    let mut community = Community::create_on(start_with(n, seed, &|i| (mode(i), None)).await).await;
    for i in 1..n {
        community.join(i).await;
    }
    community
}

/// The pseudonyms of every node but `i`.
async fn others(community: &Community, i: usize) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for x in (0..community.nodes.len()).filter(|x| *x != i) {
        set.insert(community.pseudonym(x).await);
    }
    set
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_shell_member_and_a_daemon_member_reach_each_other_by_m6() {
    let community = with_shell_member(2, 1, 5521).await;
    for i in 0..2 {
        let want = others(&community, i).await;
        let governance = community.nodes[i].host.governance();
        let routed = || {
            let routed: BTreeSet<String> = governance
                .routed_members(&community.anchor)
                .into_iter()
                .map(|(p, _)| p)
                .collect();
            let mesh: BTreeSet<String> = governance
                .mesh_peers(&community.anchor)
                .into_iter()
                .map(|p| p.pseudonym_key)
                .collect();
            routed == want && mesh == want
        };
        assert!(
            eventually(&community.sim, Duration::from_secs(120), || async {
                routed()
            })
            .await,
            "node {i} ({:?}) holds the other member's M/6 route and meshes with them",
            community.nodes[i].mode
        );
    }
}

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

/// Node 0 kicks `target`'s member from the community.
async fn kick(community: &Community, target: usize) {
    let target_pseudonym = community.pseudonym(target).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym,
                reason: None,
            },
        )))
        .await;
}

/// The step's Verify on the shell mode: a rotation's push reaches the
/// shell member by its `M`/6 route, and so does the answer to the
/// `RequestMEK` it sends for a generation it missed while cut off (the
/// S7m 1c repair, `mek_recovery.rs`): nobody pushes that key again.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_rotation_and_a_request_mek_answer_reach_a_shell_member() {
    let community = with_shell_member(4, 1, 5522).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let before = community_generation(&community, 1).await;
    kick(&community, 3).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community_generation(&community, 1).await > before
        })
        .await,
        "the shell member receives the rotated key over its M/6 route"
    );
    let pushed = community_generation(&community, 1).await;
    community.sim.fault(Fault::Partition {
        a: vec![0, 2, 3],
        b: vec![1],
    });
    kick(&community, 2).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community_generation(&community, 0).await > pushed
        })
        .await,
        "node 0 rotates past the generation pushed to the shell member"
    );
    assert_eq!(
        community_generation(&community, 1).await,
        pushed,
        "the shell member was cut off for the second rotation"
    );
    community.sim.fault(Fault::Heal);
    let rotated = community_generation(&community, 0).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community_generation(&community, 1).await == rotated
        })
        .await,
        "the shell member asks for the key it missed and is answered over its M/6 route: {}",
        community.mek_list(1).await
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn online_members_are_one_source_on_both_modes() {
    let community = with_shell_member(3, 1, 5523).await;
    for i in 0..3 {
        let want = others(&community, i).await;
        let governance = community.nodes[i].host.governance();
        let agrees = || {
            let online: BTreeSet<String> = governance
                .online_members(&community.anchor)
                .into_iter()
                .map(|m| m.pseudonym_hex)
                .collect();
            let shown: BTreeSet<String> = want
                .iter()
                .filter(|p| {
                    governance
                        .member_beat_shown(&community.anchor, p)
                        .is_present()
                })
                .cloned()
                .collect();
            online == want && shown == want
        };
        assert!(
            eventually(&community.sim, Duration::from_secs(180), || async {
                agrees()
            })
            .await,
            "node {i} ({:?}) lists every present member online, as its beat view shows",
            community.nodes[i].mode
        );
    }
}
