//! D15's convergence check (plan E3.2 Verify): two members holding one op
//! set compute byte-identical roster and role tables, checked by hash (Tox
//! `peers_checksum`; `rekindle-governance/src/checksum`). Each node merges
//! its own op store, received over its own paths and in its own order; once
//! every node holds the same op hashes, every node's roster checksum (the
//! one its `M`/3 frontier publishes, `op_log.rs` `own_frontier`), and its
//! roster, role and assignment tables, are equal.

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_governance::checksum::roster_checksum;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_types::governance::OpHash;
use rekindle_types::permissions::KICK_MEMBERS;

use crate::harness::{eventually, Community};

async fn op_set(community: &Community, i: usize) -> BTreeSet<OpHash> {
    (community.nodes[i].host.governance())
        .governance_ops(&community.anchor)
        .await
        .map(|held| held.into_iter().map(|op| op.hash).collect())
        .unwrap_or_default()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn members_holding_one_op_set_have_equal_roster_and_role_hashes() {
    let community = Community::of(3, 4301).await;
    let bea = community.pseudonym(1).await;
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: community.anchor.clone(),
                name: "mod".into(),
                permissions: KICK_MEMBERS,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )))
        .await;
    let role_id = role.as_str().expect("a role id").to_owned();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleAssign {
                community: community.anchor.clone(),
                member_pseudonym: bea,
                role_id: role_id.clone(),
            },
        )))
        .await;
    assert!(
        community.holds_role(1, &role_id).await,
        "bea holds the role"
    );

    let converged = eventually(&community.sim, Duration::from_secs(300), || async {
        let first = op_set(&community, 0).await;
        let mut same = !first.is_empty();
        for i in 1..community.nodes.len() {
            same &= op_set(&community, i).await == first;
        }
        same
    })
    .await;
    assert!(converged, "every member holds one op set");

    let states: Vec<_> = (community.nodes.iter())
        .map(|node| {
            (node.host.governance())
                .governance_state(&community.anchor)
                .expect("a merged state")
        })
        .collect();
    let checksums: Vec<_> = states.iter().map(roster_checksum).collect();
    assert!(
        checksums.windows(2).all(|w| w[0] == w[1]),
        "one op set, one roster checksum: {checksums:?}"
    );
    for state in &states[1..] {
        assert_eq!(state.members, states[0].members, "one roster");
        assert_eq!(state.roles, states[0].roles, "one role table");
        assert_eq!(
            state.role_assignments, states[0].role_assignments,
            "one assignment"
        );
    }
}
