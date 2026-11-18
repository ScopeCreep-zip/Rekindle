//! A governance write that runs on a task other work waits behind takes
//! the queued path (plan E3.2 S6b): the governance-op outbox stream lands
//! it, so the task never waits on the network. While the creator's op log
//! cannot land (`Fault::WriteFails` on its member record), the role it
//! assigns on a member's onboarding answers (the control handler, awaited
//! inline) reaches the member at once.

use std::time::Duration;

use rekindle_ipc::protocol::{
    ChannelMessageRequest, CommunityRequest, IpcRequest, LifecycleRequest,
};
use rekindle_ipc::protocol::{MemberRequest, SettingsRequest};
use rekindle_netsim::{Fault, Writes};
use rekindle_types::dto::onboarding::{
    OnboardingConfigDto, OnboardingModeDto, OnboardingOptionDto, OnboardingQuestionDto,
};
use rekindle_types::presence::OnboardingAnswer;

use crate::harness::{eventually, Community};
use crate::invite_unpublished::creator_record;

fn creator_cannot_land(community: &Community) {
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(creator_record(community)),
    });
}

fn prompt(role_id: u32) -> OnboardingConfigDto {
    OnboardingConfigDto {
        enabled: true,
        mode: OnboardingModeDto::Guided,
        default_channels: Vec::new(),
        questions: vec![OnboardingQuestionDto {
            question_id: "q1".into(),
            title: "Why?".into(),
            description: None,
            required: true,
            single_select: true,
            options: vec![OnboardingOptionDto {
                option_id: "raid".into(),
                title: "Raiding".into(),
                description: None,
                roles_to_assign: vec![role_id],
                channels_to_show: Vec::new(),
            }],
        }],
        welcome_message: None,
        guide_steps: Vec::new(),
    }
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
    let role_id = u32::try_from(role.as_u64().expect("a role id")).unwrap();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::OnboardingConfigSet {
                community: key.clone(),
                config: prompt(role_id),
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
                answers: vec![OnboardingAnswer {
                    question_id: "q1".into(),
                    selected_options: vec!["raid".into()],
                }],
                acknowledged_rules: false,
            },
        )))
        .await;
    let granted = eventually(&community.sim, Duration::from_secs(60), || async {
        community.holds_role(1, role_id).await
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
/// log unable to land, two rotations in a row finish on every node before
/// one network answer's deadline (S6b).
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
    let accepted = eventually(
        &community.sim,
        rekindle_types::config::NETWORK_ANSWER_DEADLINE,
        || async { generations(&community).await == vec![3, 3, 3] },
    )
    .await;
    assert!(
        accepted,
        "both bumps on every reader: {:?}",
        generations(&community).await
    );
}
