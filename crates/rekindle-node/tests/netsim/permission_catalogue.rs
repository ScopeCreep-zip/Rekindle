use std::time::Duration;

use rekindle_ipc::protocol::{
    CommunityRequest, IpcRequest, IpcResponse, LifecycleRequest, MemberRequest,
};
use rekindle_ipc::{decode_frame, encode_frame};
use rekindle_types::dto::permission_catalogue::PermissionCatalogue;
use rekindle_types::permissions::PERMISSION_CATALOGUE;
use rekindle_types::subscription_events::GovernanceEvent;

use crate::harness::{eventually, start_with, Community};
use crate::shell_node::SimMode;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn both_locked_host_modes_serve_the_types_permission_catalogue() {
    let (_sim, nodes) = start_with(2, 584, &|index| {
        ([SimMode::Daemon, SimMode::Shell][index], None)
    })
    .await;
    let request = encode_frame(&IpcRequest::PermissionCatalogue).unwrap();

    for node in nodes {
        let response = node.answer(decode_frame(&request).unwrap()).await;
        let wire = serde_json::to_vec(&response).unwrap();
        let IpcResponse::Ok {
            data, undo: None, ..
        } = serde_json::from_slice(&wire).unwrap()
        else {
            panic!("expected a static catalogue, got {response:?}");
        };
        let catalogue: PermissionCatalogue = serde_json::from_value(data).unwrap();
        assert_eq!(catalogue.groups.len(), PERMISSION_CATALOGUE.len());
        for (actual, expected) in catalogue.groups.iter().zip(PERMISSION_CATALOGUE) {
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.permissions.len(), expected.permissions.len());
            for (actual, expected) in actual.permissions.iter().zip(expected.permissions) {
                assert_eq!(actual.key, expected.key);
                assert_eq!(actual.label, expected.label);
                assert_eq!(actual.bit, expected.bit.to_string());
            }
        }
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn host_serves_owner_position_label_in_queries_and_role_events() {
    let community = Community::create(1, 585).await;
    let node = &community.nodes[0];
    let events = node.events().await;
    let role = node
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: community.anchor.clone(),
                name: "Raiders".into(),
                permissions: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )))
        .await;
    let created = role.as_str().expect("the new role id");

    let views = node
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        )))
        .await;
    let list = node
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleList {
                community: community.anchor.clone(),
            },
        )))
        .await;
    for roles in [&views[0]["roles"], &list] {
        let roles = roles.as_array().expect("roles in the Host response");
        let owner = roles.iter().find(|role| role["name"] == "Owner").unwrap();
        assert_eq!(owner["position"], u32::MAX);
        assert_eq!(owner["positionLabel"], "Always first");
        let role = roles.iter().find(|role| role["id"] == created).unwrap();
        assert_eq!(role["positionLabel"], role["position"].to_string());
    }

    assert!(
        eventually(&community.sim, Duration::from_secs(30), || async {
            events.governance().iter().any(|event| {
                matches!(event, GovernanceEvent::RolesChanged { roles, .. }
                    if roles.iter().any(|role| role.id == created)
                        && roles.iter().any(|role| role.name == "Owner"
                            && role.position_label == "Always first"))
            })
        })
        .await,
        "the role-table event carries the owner's position label"
    );
}
