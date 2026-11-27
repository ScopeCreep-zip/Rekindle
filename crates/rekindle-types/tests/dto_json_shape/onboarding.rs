//! The onboarding flow, welcome screen and member view samples in the
//! golden DTO file (plan E3.1 D3, E3.2 S9).

use rekindle_types::dto::community::{CommunityMemberView, MemberGroup, MemberModeration};
use rekindle_types::dto::onboarding::{
    GuideStepDto, OnboardingConfigDto, OnboardingModeDto, OnboardingOptionDto,
    OnboardingQuestionDto, WelcomeChannelEntryDto, WelcomeScreenDto,
};
use serde_json::Value;

use super::{pk, to};

/// The onboarding flow, welcome screen and member view the Host answers
/// (plan E3.1 D3); `roleIds` is what an onboarding grant changes.
pub(super) fn samples(add: &mut impl FnMut(&str, Value)) {
    let channel = "0c".repeat(16);
    add(
        "onboarding.config.guided",
        to(OnboardingConfigDto {
            enabled: true,
            mode: OnboardingModeDto::Guided,
            default_channels: vec![channel.clone()],
            questions: vec![OnboardingQuestionDto {
                question_id: "q_01".into(),
                title: "What do you play?".into(),
                description: Some("Pick one".into()),
                required: true,
                single_select: true,
                options: vec![OnboardingOptionDto {
                    option_id: "opt_fps".into(),
                    title: "Shooters".into(),
                    description: None,
                    roles_to_assign: vec!["05".repeat(16)],
                    channels_to_show: vec![channel.clone()],
                }],
            }],
            welcome_message: Some("Welcome!".into()),
            guide_steps: vec![GuideStepDto {
                title: "Say hello".into(),
                description: "Introduce yourself".into(),
                channel_id: Some(channel.clone()),
                emoji: Some("👋".into()),
            }],
        }),
    );
    add(
        "onboarding.config.default",
        to(OnboardingConfigDto::default()),
    );
    add(
        "onboarding.welcomeScreen",
        to(WelcomeScreenDto {
            description: "LAN party HQ".into(),
            channels: vec![WelcomeChannelEntryDto {
                channel_id: channel,
                description: "Chat with everyone".into(),
                emoji: None,
            }],
        }),
    );
    add(
        "community.denied",
        to(
            rekindle_types::dto::community_capability::DeniedCapabilities::from([
                (
                    rekindle_types::dto::community_capability::CommunityCapability::ManageChannels,
                    "You need the Manage Channels permission.".to_owned(),
                ),
                (
                    rekindle_types::dto::community_capability::CommunityCapability::OfferSuccessor,
                    "Only a creator of this community can offer a successor community.".to_owned(),
                ),
            ]),
        ),
    );
    add(
        "community.memberView",
        to(CommunityMemberView {
            pseudonym_key: pk('b'),
            display_name: "Bea".into(),
            fingerprint: "bbbb bbbb".into(),
            custom_status: Some("brb".into()),
            role_ids: vec!["00".repeat(16), "05".repeat(16)],
            display_role: "Shooters".into(),
            status: "online".into(),
            timeout_until: None,
            bio: None,
            pronouns: None,
            theme_color: None,
            badges: Vec::new(),
            avatar_ref: None,
            banner_ref: None,
            location: None,
            last_active: 0,
            group: MemberGroup::Role {
                id: "05".repeat(16),
                name: "Shooters".into(),
            },
            moderation: MemberModeration {
                kick: None,
                ban: Some("You need the Ban Members permission.".into()),
                timeout: None,
                manage_roles: Some(
                    "You can't moderate someone at or above your highest role.".into(),
                ),
                mute: Some("You need the Mute Members permission.".into()),
                deafen: None,
            },
            outranks: true,
        }),
    );
}
