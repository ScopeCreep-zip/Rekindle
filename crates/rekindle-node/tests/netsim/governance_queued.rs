//! A governance write that runs on a task other work waits behind takes
//! the queued path (plan E3.2 S6b): the governance-op outbox stream lands
//! it, so the task never waits on the network. While the creator's op log
//! cannot land (`Fault::WriteFails` on its member record), the role it
//! assigns on a member's onboarding answers (the control handler, awaited
//! inline) stands in the creator's state at once, and reaches the member
//! once it lands (lead ruling 2026-11-26 items 1 and 4).

use std::time::Duration;

use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{
    ChannelMessageRequest, CommunityRequest, IpcRequest, LifecycleRequest,
};
use rekindle_ipc::protocol::{MemberRequest, SettingsRequest};
use rekindle_netsim::{Fault, Writes};

use crate::harness::{eventually, Community};
use crate::invite_unpublished::creator_record;
use crate::onboarding::{first_option_answer, prompt};

fn creator_cannot_land(community: &Community) {
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(creator_record(community)),
    });
}

/// The creator assigns the answered role from its control handler; with
/// its op log unable to land, the assignment is queued, not waited for, and
/// the member holds the role at once.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn onboarding_answers_never_wait_on_the_network() {
    let community = Community::of(2, 231).await;
    let key = community.anchor.clone();
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: key.clone(),
                name: "raiders".into(),
                permissions: 0,
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
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::OnboardingConfigSet {
                community: key.clone(),
                config: prompt(&role_id),
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            community.nodes[1]
                .ok(IpcRequest::Community(CommunityRequest::Settings(
                    SettingsRequest::OnboardingConfigGet {
                        community: key.clone(),
                    },
                )))
                .await["questions"]
                .as_array()
                .is_some_and(|q| q.len() == 1)
        })
        .await,
        "the member reads the prompt"
    );

    creator_cannot_land(&community);
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::OnboardingSubmit {
                community: key.clone(),
                answers: vec![first_option_answer(&community, 1, &key).await],
                acknowledged_rules: false,
            },
        )))
        .await;
    // The creator's handler answered without the landing: its own state
    // holds the assignment while its op log cannot land (lead ruling
    // 2026-11-26 item 4). The member merges it only once it is readable
    // on the creator's op log (item 1), so the log heals before the
    // member's check: the old setup had the member merge an op that had
    // not landed, which the ruling removes.
    let member = community.pseudonym(1).await;
    assert!(
        crate::devices::sees_role(&community, 0, &member, &role_id).await,
        "the creator assigned the role at once, before it landed"
    );
    community.sim.fault(Fault::Heal);
    let granted = eventually(&community.sim, Duration::from_secs(60), || async {
        community.holds_role(1, &role_id).await
    })
    .await;
    assert!(granted, "the role assignment did not wait on the network");
}

/// Every node's merged community-key generation, from its community view.
async fn generations(community: &Community) -> Vec<u64> {
    let mut out = Vec::new();
    for node in &community.nodes {
        let views = node
            .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityViews,
            )))
            .await;
        out.push(
            views
                .as_array()
                .and_then(|views| {
                    views
                        .iter()
                        .find(|v| v["id"].as_str() == Some(community.anchor.as_str()))
                })
                .and_then(|v| v["mekGeneration"].as_u64())
                .unwrap_or_default(),
        );
    }
    out
}

/// A manual community-key rotation's `MEKGenerationBump` names the key
/// after the one minted at origin, so every reader accepts it (plan D20,
/// `current + 1`); the rotation worker queues it, so with the creator's op
/// log unable to land, neither request waits (S6b) and the first bump is
/// on the author before one network answer's deadline; every reader merges
/// it once it lands (lead ruling 2026-11-26 items 1 and 4). The second is held
/// until the first's append landed (S7d-keys.8: seq n + 1 is signed only
/// after seq n's append landed), then reaches every reader.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_manual_rotation_never_waits_and_every_reader_accepts_its_bump() {
    let community = Community::of(3, 233).await;
    assert_eq!(
        generations(&community).await,
        vec![1, 1, 1],
        "every member starts at the origin key's generation"
    );
    creator_cannot_land(&community);
    for _ in 0..2 {
        community.nodes[0]
            .ok(IpcRequest::Community(CommunityRequest::Messages(
                ChannelMessageRequest::MekRotate {
                    community: community.anchor.clone(),
                    channel: String::new(),
                },
            )))
            .await;
    }
    // The author applies its own bump at once (lead ruling 2026-11-26
    // item 4); a reader merges it only once it is readable on the
    // creator's op log (item 1), so the readers' check moves past the
    // heal: the old setup had them merge a bump that had not landed.
    let first = eventually(
        &community.sim,
        rekindle_types::config::NETWORK_ANSWER_DEADLINE,
        || async { generations(&community).await[0] == 2 },
    )
    .await;
    assert!(
        first,
        "the first bump on the author at once: {:?}",
        generations(&community).await
    );
    let barrier = (community.nodes[0].host.governance()).unlock_barrier(&community.anchor);
    assert!(
        barrier.view().is_some_and(|v| !v.pending.is_empty()),
        "the second bump is held behind the first's append"
    );
    community.sim.fault(Fault::Heal);
    let first = eventually(
        &community.sim,
        rekindle_types::config::NETWORK_ANSWER_DEADLINE * 2,
        || async { generations(&community).await.iter().all(|g| *g >= 2) },
    )
    .await;
    assert!(
        first,
        "the first bump on every reader: {:?}",
        generations(&community).await
    );
    let accepted = eventually(
        &community.sim,
        rekindle_types::config::NETWORK_ANSWER_DEADLINE * 2,
        || async { generations(&community).await == vec![3, 3, 3] },
    )
    .await;
    assert!(
        accepted,
        "both bumps on every reader: {:?}",
        generations(&community).await
    );
}
