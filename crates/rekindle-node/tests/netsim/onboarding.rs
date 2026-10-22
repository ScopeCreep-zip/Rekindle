//! Plan E3.1 D3: an onboarding prompt grants a role on both members, a later
//! unassignment wins, and a member without `MANAGE_COMMUNITY` cannot set
//! the prompt.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_ipc::protocol::{LifecycleRequest, MemberRequest, SettingsRequest};
use rekindle_types::dto::onboarding::{
    OnboardingConfigDto, OnboardingModeDto, OnboardingOptionDto, OnboardingQuestionDto,
};
use rekindle_types::presence::OnboardingAnswer;

use crate::harness::{eventually, Community};

/// The role ids node `i` lists for member `pseudonym`.
async fn member_roles(community: &Community, i: usize, pseudonym: &str) -> Vec<u64> {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .as_array()
        .and_then(|members| {
            members
                .iter()
                .find(|m| m["pseudonymKey"].as_str() == Some(pseudonym))
                .and_then(|m| m["roleIds"].as_array())
                .map(|ids| ids.iter().filter_map(serde_json::Value::as_u64).collect())
        })
        .unwrap_or_default()
}

fn prompt(role_id: u32) -> OnboardingConfigDto {
    OnboardingConfigDto {
        enabled: true,
        mode: OnboardingModeDto::Guided,
        default_channels: Vec::new(),
        questions: vec![OnboardingQuestionDto {
            question_id: "q1".into(),
            title: "What brings you here?".into(),
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
        welcome_message: Some("Welcome!".into()),
        guide_steps: Vec::new(),
    }
}

/// The creator sets a prompt whose option grants a role the members cannot
/// take themselves; a member answers it, and both members see it hold the
/// role (granted by the creator on the answers). The creator's later
/// unassignment wins on both. The member cannot set the prompt.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn onboarding_role_assignment() {
    let community = Community::of(2, 83).await;
    let bea = community.pseudonym(1).await;
    let key = community.governance_key.clone();
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: key.clone(),
                name: "raiders".into(),
                permissions: 0,
                color: 0x00ff_8800,
                hoist: true,
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
    let refused = community.nodes[1]
        .call(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::OnboardingConfigSet {
                community: key.clone(),
                config: prompt(role_id),
            },
        )))
        .await
        .expect_err("a member without MANAGE_COMMUNITY is refused");
    assert_eq!(refused.code, 403, "{}", refused.message);

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
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::OnboardingComplete {
                community: key.clone(),
            },
        )))
        .await;

    assert!(
        community.holds_role(1, role_id).await,
        "the member holds the role it answered for"
    );
    let granted = u64::from(role_id);
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            member_roles(&community, 0, &bea).await.contains(&granted)
        })
        .await,
        "the creator sees the member hold the role"
    );

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleUnassign {
                community: key.clone(),
                member_pseudonym: bea.clone(),
                role_id,
            },
        )))
        .await;
    for i in 0..2 {
        assert!(
            eventually(&community.sim, Duration::from_secs(60), || async {
                !member_roles(&community, i, &bea).await.contains(&granted)
            })
            .await,
            "the later unassignment wins on node {i}"
        );
    }
}
