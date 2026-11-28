//! Plan E3.1 D3: an onboarding prompt grants a role on both members, a later
//! unassignment wins, and a member without `MANAGE_COMMUNITY` cannot set
//! the prompt; a welcome screen set through the request the GUI's Welcome
//! tab sends is what the member reads (plan E7.15).

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_ipc::protocol::{LifecycleRequest, MemberRequest, SettingsRequest};
use rekindle_types::dto::onboarding::{
    OnboardingConfigEdit, OnboardingModeDto, OnboardingOptionEdit, OnboardingQuestionEdit,
    WelcomeScreenDto,
};
use rekindle_types::presence::OnboardingAnswer;

use crate::harness::{eventually, Community};

/// The role ids node `i` lists for member `pseudonym`.
async fn member_roles(community: &Community, i: usize, pseudonym: &str) -> Vec<String> {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                anchor: community.anchor.clone(),
            },
        )))
        .await
        .as_array()
        .and_then(|members| {
            members
                .iter()
                .find(|m| m["pseudonymKey"].as_str() == Some(pseudonym))
                .and_then(|m| m["roleIds"].as_array())
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| id.as_str().map(str::to_owned))
                        .collect()
                })
        })
        .unwrap_or_default()
}

/// A one-question prompt as the editor sends it: the question and its
/// option are new, so they carry no id and the Host mints them.
pub(crate) fn prompt(role_id: &str) -> OnboardingConfigEdit {
    OnboardingConfigEdit {
        enabled: true,
        mode: OnboardingModeDto::Guided,
        default_channels: Vec::new(),
        questions: vec![OnboardingQuestionEdit {
            question_id: None,
            title: "What brings you here?".into(),
            description: None,
            required: true,
            single_select: true,
            options: vec![OnboardingOptionEdit {
                option_id: None,
                title: "Raiding".into(),
                description: None,
                roles_to_assign: vec![role_id.to_owned()],
                channels_to_show: Vec::new(),
            }],
        }],
        welcome_message: Some("Welcome!".into()),
        guide_steps: Vec::new(),
    }
}

/// Node `i`'s answer choosing the first option of the prompt's first
/// question, by the ids the Host minted (read from its merged flow).
pub(crate) async fn first_option_answer(
    community: &Community,
    i: usize,
    key: &str,
) -> OnboardingAnswer {
    let flow = community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::OnboardingConfigGet {
                community: key.to_owned(),
            },
        )))
        .await;
    let question = &flow["questions"][0];
    OnboardingAnswer {
        question_id: question["questionId"]
            .as_str()
            .expect("a minted id")
            .to_owned(),
        selected_options: vec![question["options"][0]["optionId"]
            .as_str()
            .expect("a minted id")
            .to_owned()],
    }
}

/// The creator sets a prompt whose option grants a role the members cannot
/// take themselves; a member answers it, and both members see it hold the
/// role (granted by the creator on the answers). The creator's later
/// unassignment wins on both. The member cannot set the prompt.
///
/// Revert run (review RV7 L3, 2026-11-14): with 1f06f6a4's `views.rs`
/// reverted (the member list's role ids read from the roster row's cache)
/// it fails at "the later unassignment wins on node 1".
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn onboarding_role_assignment() {
    let community = Community::of(2, 83).await;
    let bea = community.pseudonym(1).await;
    let key = community.anchor.clone();
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
    let role_id = role.as_str().expect("a role id").to_owned();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::OnboardingConfigSet {
                community: key.clone(),
                config: prompt(&role_id),
            },
        )))
        .await;
    let refused = community.nodes[1]
        .call(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::OnboardingConfigSet {
                community: key.clone(),
                config: prompt(&role_id),
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
                answers: vec![first_option_answer(&community, 1, &key).await],
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
        community.holds_role(1, &role_id).await,
        "the member holds the role it answered for"
    );
    let granted = role_id.clone();
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
                role_id: role_id.clone(),
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

/// The Welcome tab's save (`set_welcome_screen` → `WelcomeScreenSet`) and the
/// joiner's read (`get_welcome_screen` → `WelcomeScreenGet`): the screen the
/// creator sets is the one the member reads, and the creator reads it back.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn welcome_screen_round_trip() {
    let community = Community::of(2, 84).await;
    let key = community.anchor.clone();
    let screen = WelcomeScreenDto {
        description: "LAN party HQ".into(),
        channels: Vec::new(),
    };
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::WelcomeScreenSet {
                community: key.clone(),
                screen: screen.clone(),
            },
        )))
        .await;
    for i in [0, 1] {
        assert!(
            eventually(&community.sim, Duration::from_secs(60), || async {
                let read = community.nodes[i]
                    .ok(IpcRequest::Community(CommunityRequest::Settings(
                        SettingsRequest::WelcomeScreenGet {
                            community: key.clone(),
                        },
                    )))
                    .await;
                serde_json::from_value::<WelcomeScreenDto>(read).ok() == Some(screen.clone())
            })
            .await,
            "node {i} reads the welcome screen that was set"
        );
    }
}
