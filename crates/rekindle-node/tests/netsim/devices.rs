//! Plan E3.2 S7d-keys K5 (ADR 0022 rules 3, 4 and 10; lead decision 63): a
//! person's second device. Person `p` (node 1, device A) restores a backup
//! of its data root on another machine (node 2, device B): the data root
//! and Veilid's local store, without the device vault no backup carries
//! (rule 4), so B holds the community's state and no device key. B enrols
//! a fresh key (`CommunityDeviceEnrol`, `p`'s `DeviceKey` as the key's
//! seq-0 op on its own `M(p,d)`) and writes an op the other members see as
//! `p`'s. A then leaves and re-joins: B's endorsement binds the ended member
//! record (rule 10), so after B's next unlock catch-up its view reads
//! `NeedsReenrol` and its writes are refused locally (`DeviceUnendorsed`).
//! B re-enrols with a fresh key and writes again.
//!
//! Readers refusing an op a stale device signed anyway is K4b (the device
//! judge in the merge); this scenario asserts the writer-side refusal only.

use std::time::Duration;

use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest, MemberRequest};

use crate::harness::{eventually, Community, Refused, PASSPHRASE};

const LIMIT: Duration = Duration::from_secs(180);
/// Device A, `p`'s first device.
const A: usize = 1;
/// Device B, `p`'s second device (node 2's own identity is never used).
const B: usize = 2;

pub(crate) async fn unlock(community: &Community, i: usize) {
    let node = &community.nodes[i];
    node.ok(IpcRequest::Unlock {
        identity: node.public_key.clone().expect("an identity"),
        passphrase: PASSPHRASE.into(),
    })
    .await;
}

pub(crate) async fn self_assignable_role(community: &Community, name: &str) -> String {
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: community.anchor.clone(),
                name: name.into(),
                permissions: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: true,
                exclusion_group: None,
            },
        )))
        .await;
    role.as_str().expect("a role id").to_owned()
}

/// Node `i` takes `role_id` itself: a governance op of its own.
pub(crate) async fn self_assign(
    community: &Community,
    i: usize,
    role_id: &str,
) -> Result<serde_json::Value, Refused> {
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleSelfAssign {
                community: community.anchor.clone(),
                role_id: role_id.to_owned(),
            },
        )))
        .await
}

/// Whether node `reader` lists `member` holding `role_id`, within the limit.
pub(crate) async fn sees_role(
    community: &Community,
    reader: usize,
    member: &str,
    role_id: &str,
) -> bool {
    eventually(&community.sim, LIMIT, || async {
        community.nodes[reader]
            .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityMembers {
                    anchor: community.anchor.clone(),
                },
            )))
            .await
            .ok()
            .and_then(|v| v.as_array().cloned())
            .is_some_and(|rows| {
                rows.iter().any(|m| {
                    m["pseudonymKey"].as_str() == Some(member)
                        && m["roleIds"]
                            .as_array()
                            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(role_id)))
                })
            })
    })
    .await
}

/// Whether node `i`'s view of the community reads `status` within the limit.
pub(crate) async fn device_status(community: &Community, i: usize, status: &str) -> bool {
    eventually(&community.sim, LIMIT, || async {
        community.nodes[i]
            .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityViews,
            )))
            .await
            .ok()
            .and_then(|views| views.as_array().cloned())
            .is_some_and(|views| {
                views.iter().any(|v| {
                    v["id"].as_str() == Some(community.anchor.as_str())
                        && v["deviceStatus"].as_str() == Some(status)
                })
            })
    })
    .await
}

/// Whether node `reader` lists `member`, within the limit.
pub(crate) async fn lists(
    community: &Community,
    reader: usize,
    member: &str,
    listed: bool,
) -> bool {
    eventually(&community.sim, LIMIT, || async {
        community.nodes[reader]
            .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityMembers {
                    anchor: community.anchor.clone(),
                },
            )))
            .await
            .ok()
            .and_then(|v| v.as_array().cloned())
            .is_some_and(|rows| {
                rows.iter()
                    .any(|m| m["pseudonymKey"].as_str() == Some(member))
                    == listed
            })
    })
    .await
}

async fn enrol(community: &Community, i: usize) {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityDeviceEnrol {
                anchor: community.anchor.clone(),
            },
        )))
        .await;
}

/// B: `p`'s data root restored on node 2 without the device vault, the
/// Host started over it through the production startup path, unlocked.
async fn second_device(community: &mut Community) {
    community.nodes[B].ok(IpcRequest::Lock).await;
    community.nodes[A].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[A]);
    unlock(community, A).await;
    let p_key = community.nodes[A].public_key.clone().expect("p's identity");
    crate::data_root::restore_as_new_device(&community.sim, &community.nodes[B], &backup, &p_key);
    let node = community.nodes.pop().expect("node 2");
    let mut node = node.restart(&community.sim, Duration::from_secs(1)).await;
    node.public_key = Some(p_key);
    community.nodes.push(node);
    unlock(community, B).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_second_device_enrols_then_re_enrols_after_its_person_rejoins() {
    let mut community = Community::create(3, 2207).await;
    community.join(A).await;
    let p = community.pseudonym(A).await;
    let first = self_assignable_role(&community, "first").await;
    let second = self_assignable_role(&community, "second").await;
    let third = self_assignable_role(&community, "third").await;
    tokio::time::sleep(Duration::from_secs(60)).await;

    second_device(&mut community).await;
    assert_eq!(community.pseudonym(B).await, p, "B is p in the community");
    assert!(
        device_status(&community, B, "needsReenrol").await,
        "a device new to the identity holds no endorsed key"
    );
    enrol(&community, B).await;
    assert!(
        device_status(&community, B, "enrolled").await,
        "the enrolled device reads Enrolled"
    );
    self_assign(&community, B, &first)
        .await
        .expect("the enrolled device writes");
    assert!(
        sees_role(&community, 0, &p, &first).await,
        "another member sees B's op as p's"
    );

    // B goes off-line once its op landed on its `M(p,d)`: a write made
    // while the previous op's append is still in flight is held and
    // answered at once (plan E3.2 S7d-keys.8), and an op that never landed
    // is not on the record a later joiner walks.
    let barrier = community.nodes[B]
        .host
        .governance()
        .unlock_barrier(&community.anchor);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            tokio::time::timeout(Duration::ZERO, barrier.landed())
                .await
                .is_ok()
        })
        .await,
        "B's op lands on its own record"
    );
    // B goes off-line; A leaves and re-joins under a fresh member record.
    community.nodes[B].ok(IpcRequest::Lock).await;
    community.nodes[A]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                anchor: community.anchor.clone(),
            },
        )))
        .await
        .expect("A's leave commits");
    assert!(lists(&community, 0, &p, false).await, "p is gone");
    community.join(A).await;
    assert!(lists(&community, 0, &p, true).await, "p is back");
    self_assign(&community, A, &second)
        .await
        .expect("A writes under its re-join");
    assert!(sees_role(&community, 0, &p, &second).await);

    // B's next unlock catches up: its key binds the ended member record.
    unlock(&community, B).await;
    assert!(
        device_status(&community, B, "needsReenrol").await,
        "B's endorsement bound the ended member record"
    );
    let refused = self_assign(&community, B, &third)
        .await
        .expect_err("the stale device's write is refused locally");
    assert!(
        refused.message.contains("op key is not endorsed"),
        "refused as DeviceUnendorsed: {refused:?}"
    );

    enrol(&community, B).await;
    assert!(
        device_status(&community, B, "enrolled").await,
        "the re-enrolled device reads Enrolled"
    );
    self_assign(&community, B, &third)
        .await
        .expect("the re-enrolled device writes");
    assert!(
        sees_role(&community, 0, &p, &third).await,
        "another member sees B's new op as p's"
    );
}
