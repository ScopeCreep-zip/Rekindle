//! A governance write that runs on a task other work waits behind takes
//! the queued path (plan E3.2 S6b): the governance-op outbox stream lands
//! it, so the task never waits on the network. While the creator's op log
//! cannot land (`Fault::WriteFails` on its member record), the role it
//! assigns on a member's onboarding answers (the control handler, awaited
//! inline) reaches the member at once.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
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
    let key = community.governance_key.clone();
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
