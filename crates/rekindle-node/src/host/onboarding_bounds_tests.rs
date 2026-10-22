//! The onboarding bounds at write (plan E3.1 D3): a flow or welcome screen
//! readers would drop is refused by field, `OutOfBounds`, answered 400.

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_transport::session::CommunityMembership;
use rekindle_types::dto::onboarding::{
    OnboardingConfigDto, OnboardingQuestionDto, WelcomeChannelEntryDto, WelcomeScreenDto,
};
use rekindle_types::governance::{
    MAX_ONBOARDING_QUESTIONS, MAX_ONBOARDING_WELCOME_CHARS, MAX_WELCOME_SCREEN_CHANNELS,
};

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::governance::onboarding::OnboardingRefusal;

fn question(n: usize) -> OnboardingQuestionDto {
    OnboardingQuestionDto {
        question_id: format!("q_{n}"),
        title: "What do you play?".into(),
        description: None,
        required: false,
        single_select: true,
        options: Vec::new(),
    }
}

fn channel() -> WelcomeChannelEntryDto {
    WelcomeChannelEntryDto {
        channel_id: "0c".repeat(16),
        description: "Chat with everyone".into(),
        emoji: None,
    }
}

/// Ann's membership of `c1`, so a write takes its next Lamport clock.
fn load_membership(host: &crate::host::Host) {
    let membership = CommunityMembership {
        governance_key: "c1".into(),
        pseudonym_key: "cd".repeat(32),
        role_ids: vec![0],
        registry_key: "registry".into(),
        slot_index: 0,
        community_name: "C".into(),
        slot_seed: None,
        channel_record_keys: std::collections::HashMap::new(),
        segment_index: None,
        lamport_counter: 1,
        mek_generation: 0,
    };
    let mut session = host.ctx.session.write();
    let session = session.as_mut().expect("Ann is unlocked");
    session.communities.insert("c1".into(), membership);
}

/// The field a refusal names, after checking it answers 400.
fn out_of_bounds(refused: Result<(), OnboardingRefusal>) -> &'static str {
    let refused = refused.unwrap_err();
    assert_eq!(refused.code(), 400, "{refused}");
    match refused {
        OnboardingRefusal::Failed(GovernanceRuntimeError::OutOfBounds { field }) => field,
        other => panic!("refused with {other}"),
    }
}

#[tokio::test]
async fn an_out_of_bounds_flow_or_screen_is_refused_at_write() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    load_membership(&host);
    let governance = host.governance();

    let too_many = OnboardingConfigDto {
        enabled: true,
        questions: (0..=MAX_ONBOARDING_QUESTIONS).map(question).collect(),
        ..OnboardingConfigDto::default()
    };
    let field = out_of_bounds(governance.set_onboarding_config("c1", &too_many).await);
    assert_eq!(field, "questions");

    let long_welcome = OnboardingConfigDto {
        enabled: true,
        welcome_message: Some("x".repeat(MAX_ONBOARDING_WELCOME_CHARS + 1)),
        ..OnboardingConfigDto::default()
    };
    let field = out_of_bounds(governance.set_onboarding_config("c1", &long_welcome).await);
    assert_eq!(field, "welcome_message");

    let crowded = WelcomeScreenDto {
        description: "LAN party HQ".into(),
        channels: (0..=MAX_WELCOME_SCREEN_CHANNELS)
            .map(|_| channel())
            .collect(),
    };
    let field = out_of_bounds(governance.set_welcome_screen("c1", &crowded).await);
    assert_eq!(field, "channels");

    let hidden = WelcomeScreenDto {
        description: "bell\u{7}".into(),
        channels: vec![channel()],
    };
    let field = out_of_bounds(governance.set_welcome_screen("c1", &hidden).await);
    assert_eq!(field, "description");
    host.shutdown().await.unwrap();
}
