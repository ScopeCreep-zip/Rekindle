//! The onboarding bounds at write (plan E3.1 D3): a flow or welcome screen
//! readers would drop is refused by field, `OutOfBounds`, answered 400; and
//! the Host mints a new question's or option's id (plan E7.15 S3).

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_transport::session::CommunityMembership;
use rekindle_types::dto::onboarding::{
    OnboardingConfigEdit, OnboardingOptionEdit, OnboardingQuestionEdit, WelcomeChannelEntryDto,
    WelcomeScreenDto,
};
use rekindle_types::governance::{
    MAX_ONBOARDING_QUESTIONS, MAX_ONBOARDING_WELCOME_CHARS, MAX_WELCOME_SCREEN_CHANNELS,
};

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::governance::onboarding::OnboardingRefusal;

fn question(_: usize) -> OnboardingQuestionEdit {
    OnboardingQuestionEdit {
        question_id: None,
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
pub(super) fn load_membership(host: &crate::host::Host) {
    let membership = CommunityMembership {
        anchor: "c1".into(),
        pseudonym_key: "cd".repeat(32),
        registry_key: "registry".into(),
        slot_index: 0,
        community_name: "C".into(),
        slot_seed: None,
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

    let too_many = OnboardingConfigEdit {
        enabled: true,
        questions: (0..=MAX_ONBOARDING_QUESTIONS).map(question).collect(),
        ..OnboardingConfigEdit::default()
    };
    let field = out_of_bounds(governance.set_onboarding_config("c1", &too_many).await);
    assert_eq!(field, "questions");

    let long_welcome = OnboardingConfigEdit {
        enabled: true,
        welcome_message: Some("x".repeat(MAX_ONBOARDING_WELCOME_CHARS + 1)),
        ..OnboardingConfigEdit::default()
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

/// A new question and option carry no id: the Host mints each (16 random
/// bytes as hex, distinct), an edit of the merged flow keeps its ids, and an
/// id the merged flow does not hold is refused before anything is written,
/// so no caller picks an id. (The write itself is netsim
/// `onboarding_role_assignment`, which answers by the minted ids.)
#[tokio::test]
async fn the_host_mints_new_onboarding_ids_and_refuses_caller_ids() {
    use crate::host::adapters::governance::onboarding_map::{config_entry, mint_id, unknown_id};
    use rekindle_types::governance::GovernanceEntry;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    load_membership(&host);
    let governance = host.governance();

    let option = || OnboardingOptionEdit {
        option_id: None,
        title: "Raiding".into(),
        description: None,
        roles_to_assign: Vec::new(),
        channels_to_show: Vec::new(),
    };
    let mut asked = question(0);
    asked.options = vec![option(), option()];
    let fresh = OnboardingConfigEdit {
        enabled: true,
        questions: vec![asked, question(1)],
        ..OnboardingConfigEdit::default()
    };
    assert_eq!(unknown_id(&fresh, None), None, "new items name no id");
    let rng = rekindle_utils::OsRandom;
    let entry = config_entry(&fresh, &mut || mint_id(&rng)).unwrap();
    let GovernanceEntry::OnboardingConfig { questions, .. } = entry else {
        panic!("not an onboarding entry");
    };
    let mut ids: Vec<&str> = questions.iter().map(|q| q.question_id.as_str()).collect();
    ids.extend(questions[0].options.iter().map(|o| o.option_id.as_str()));
    for id in &ids {
        assert_eq!(id.len(), 32, "{id}");
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4, "every minted id is distinct");

    let mut forged = fresh.clone();
    forged.questions[0].question_id = Some("caller-minted".into());
    let refused = governance
        .set_onboarding_config("c1", &forged)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), 400, "{refused}");
    assert!(
        matches!(refused, OnboardingRefusal::Invalid(_)),
        "{refused}"
    );
    let mut forged_option = fresh.clone();
    forged_option.questions[0].options[0].option_id = Some("caller-minted".into());
    let refused = governance
        .set_onboarding_config("c1", &forged_option)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, OnboardingRefusal::Invalid(_)),
        "{refused}"
    );
    host.shutdown().await.unwrap();
}
