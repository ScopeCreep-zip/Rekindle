//! Plan E3.2 S7 Verify (D15, register S7d): a restored backup equivocates.
//! The creator's data root is copied, it creates a role (an op at its
//! next seq), the copy is put back, and while it is cut off from every peer
//! (register S7f: once connected, a restore re-learns its own op from the
//! peers' frontiers before writing) it creates a different role at that
//! same seq, then the partition heals. No key is touched: the fork is what
//! a real restore made off-line does.
//! The other members detect it within one anti-entropy sweep and drop
//! every op of that author from the fork on, the later ones too.

use std::time::Duration;

use rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, Community, PASSPHRASE};

const SEEN: Duration = Duration::from_secs(180);

/// A role: a governance op and nothing else (a channel create still mints
/// its SMPL record, which a node cut off from its peers cannot store).
async fn create_role(community: &Community, i: usize, name: &str) {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: community.governance_key.clone(),
                name: name.into(),
                permissions: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )))
        .await;
}

async fn shows(community: &Community, i: usize, name: &str) -> bool {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleList {
                community: community.governance_key.clone(),
            },
        )))
        .await
        .to_string()
        .contains(name)
}

async fn unlock(community: &Community, i: usize) {
    let node = &community.nodes[i];
    node.ok(IpcRequest::Unlock {
        identity: node.public_key.clone().expect("an identity"),
        passphrase: PASSPHRASE.into(),
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_restored_backup_equivocates_and_its_later_ops_are_quarantined() {
    let community = Community::of(3, 1515).await;
    tokio::time::sleep(Duration::from_secs(60)).await;

    community.nodes[0].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[0]);
    unlock(&community, 0).await;
    create_role(&community, 0, "alpha").await;
    assert!(
        eventually(&community.sim, SEEN, || async {
            shows(&community, 1, "alpha").await && shows(&community, 2, "alpha").await
        })
        .await,
        "the members see the first branch"
    );

    community.nodes[0].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[0], &backup);
    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    unlock(&community, 0).await;
    create_role(&community, 0, "bravo").await;
    community.sim.fault(Fault::Heal);
    // One sweep period plus the time an op takes to land everywhere.
    let detected = eventually(&community.sim, SWEEP_PERIOD + SEEN, || async {
        let mut clean = true;
        for i in 1..3 {
            clean &= !shows(&community, i, "alpha").await && !shows(&community, i, "bravo").await;
        }
        clean
    })
    .await;
    assert!(
        detected,
        "both members drop the creator's ops from the fork on"
    );
    let recorded = crate::logs::warned("governance equivocation");
    for i in 1..3 {
        let node = format!("node Some({i})");
        assert!(
            recorded.iter().any(|line| line.starts_with(&node)),
            "member {i} records the equivocation: {recorded:#?}"
        );
    }

    create_role(&community, 0, "charlie").await;
    tokio::time::sleep(SWEEP_PERIOD * 2).await;
    for i in 1..3 {
        assert!(
            !shows(&community, i, "charlie").await,
            "member {i} quarantines the equivocator's later op"
        );
    }
}
