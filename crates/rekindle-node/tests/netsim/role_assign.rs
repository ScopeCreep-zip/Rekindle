//! Plan E3.1 F1: role assignment converges across members, and a member
//! without `MANAGE_ROLES` cannot grant one.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;

use crate::harness::{eventually, Community};

/// The role ids node `i` lists for the community.
async fn listed_roles(community: &Community, i: usize) -> Vec<u64> {
    community.nodes[i]
        .ok(IpcRequest::RoleList {
            community: community.governance_key.clone(),
        })
        .await
        .as_array()
        .map(|roles| roles.iter().filter_map(|r| r["id"].as_u64()).collect())
        .unwrap_or_default()
}

/// The creator makes a role and grants it to a member: the member's store
/// learns it holds the role and its role list carries it. A third member,
/// holding no `MANAGE_ROLES`, is refused when it grants the role to
/// itself, and holds nothing afterwards.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn role_assign_converges() {
    let community = Community::of(3, 71).await;
    let bea = community.pseudonym(1).await;
    let cat = community.pseudonym(2).await;
    let role = community.nodes[0]
        .ok(IpcRequest::RoleCreate {
            community: community.governance_key.clone(),
            name: "raiders".into(),
            permissions: 0,
            color: 0x00ff_8800,
            hoist: true,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        })
        .await;
    let role_id = u32::try_from(role.as_u64().expect("a role id")).unwrap();
    community.nodes[0]
        .ok(IpcRequest::RoleAssign {
            community: community.governance_key.clone(),
            member_pseudonym: bea,
            role_id,
        })
        .await;

    assert!(
        community.holds_role(1, role_id).await,
        "the member's store learns its role"
    );
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            listed_roles(&community, 1)
                .await
                .contains(&u64::from(role_id))
        })
        .await,
        "the member's role list converges"
    );

    let refused = community.nodes[2]
        .call(IpcRequest::RoleAssign {
            community: community.governance_key.clone(),
            member_pseudonym: cat,
            role_id,
        })
        .await
        .expect_err("a member without MANAGE_ROLES is refused");
    assert_eq!(refused.code, 403, "{}", refused.message);
    community.sim.quiesce().await;
    assert!(
        !community.holds_role(2, role_id).await,
        "the refused grant confers nothing"
    );
}
