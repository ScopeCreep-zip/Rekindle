//! Plan E3.1 F4: a moderation action and a role grant are rows of the audit
//! log, which a member without `VIEW_AUDIT_LOG` cannot read; the
//! notification default one member sets reaches the other.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use serde_json::Value;

use crate::harness::{eventually, Community};

fn audit_log(community: &Community) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::AuditLog {
        community: community.governance_key.clone(),
        before_timestamp: None,
        limit: 50,
    })
}

/// Whether `rows` holds `action` against `target`.
fn has_row(rows: &Value, action: &str, target: &str) -> bool {
    rows.as_array().is_some_and(|rows| {
        rows.iter()
            .any(|r| r["action"].as_str() == Some(action) && r["target"].as_str() == Some(target))
    })
}

/// The creator grants a role to one member and bans another: its audit
/// log names both, by the creator; a plain member is refused the log.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn audit_log_records_moderation_and_roles() {
    let community = Community::of(3, 101).await;
    let creator = community.pseudonym(0).await;
    let bea = community.pseudonym(1).await;
    let cat = community.pseudonym(2).await;
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::RoleCreate {
            community: community.governance_key.clone(),
            name: "raiders".into(),
            permissions: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        }))
        .await;
    let role_id = u32::try_from(role.as_u64().expect("a role id")).unwrap();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::RoleAssign {
            community: community.governance_key.clone(),
            member_pseudonym: bea.clone(),
            role_id,
        }))
        .await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Ban {
            community: community.governance_key.clone(),
            target_pseudonym: cat.clone(),
            reason: Some("spam".into()),
        }))
        .await;

    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            let rows = community.nodes[0].ok(audit_log(&community)).await;
            has_row(&rows, "role_assigned", &bea) && has_row(&rows, "member_banned", &cat)
        })
        .await,
        "the audit log names the grant and the ban"
    );
    let rows = community.nodes[0].ok(audit_log(&community)).await;
    let ban = rows
        .as_array()
        .and_then(|r| r.iter().find(|r| r["action"] == "member_banned"))
        .expect("the ban row");
    assert_eq!(ban["actorPseudonym"].as_str(), Some(creator.as_str()));
    assert_eq!(ban["details"].as_str(), Some("spam"));

    let refused = community.nodes[1]
        .call(audit_log(&community))
        .await
        .expect_err("a member without VIEW_AUDIT_LOG is refused");
    assert_eq!(refused.code, 403, "{}", refused.message);
}

/// The creator sets the community's notification default: the other member
/// reads it back, and that member, without `MANAGE_COMMUNITY`, is refused
/// when it sets one.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn notification_default_reaches_members() {
    let community = Community::of(2, 103).await;
    let get = |i: usize| {
        community.nodes[i].ok(IpcRequest::Community(
            CommunityRequest::NotificationDefaultGet {
                community: community.governance_key.clone(),
            },
        ))
    };
    assert_eq!(get(1).await, Value::Null, "no default before one is set");
    community.nodes[0]
        .ok(IpcRequest::Community(
            CommunityRequest::NotificationDefaultSet {
                community: community.governance_key.clone(),
                level: "mentions".into(),
            },
        ))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            get(1).await.as_str() == Some("mentions")
        })
        .await,
        "the member learns the default"
    );
    let refused = community.nodes[1]
        .call(IpcRequest::Community(
            CommunityRequest::NotificationDefaultSet {
                community: community.governance_key.clone(),
                level: "nothing".into(),
            },
        ))
        .await
        .expect_err("a member without MANAGE_COMMUNITY is refused");
    assert_eq!(refused.code, 403, "{}", refused.message);
}
