use super::*;

#[test]
fn onboarding_config_default() {
    let config = OnboardingConfigDto::default();
    assert!(!config.enabled);
    assert_eq!(config.mode, OnboardingModeDto::Default);
    assert!(config.questions.is_empty());
}

#[test]
fn onboarding_config_serde() {
    let config = OnboardingConfigDto {
        enabled: true,
        mode: OnboardingModeDto::Guided,
        default_channels: vec!["ch_general".into(), "ch_rules".into()],
        questions: vec![OnboardingQuestionDto {
            question_id: "q_01".into(),
            title: "What are you interested in?".into(),
            description: Some("Select your interests".into()),
            required: true,
            single_select: false,
            options: vec![
                OnboardingOptionDto {
                    option_id: "opt_gaming".into(),
                    title: "Gaming".into(),
                    description: None,
                    roles_to_assign: vec!["05".repeat(16)],
                    channels_to_show: vec!["ch_gaming".into()],
                },
                OnboardingOptionDto {
                    option_id: "opt_dev".into(),
                    title: "Development".into(),
                    description: Some("Code and tech".into()),
                    roles_to_assign: vec!["06".repeat(16)],
                    channels_to_show: vec!["ch_dev".into()],
                },
            ],
        }],
        welcome_message: Some("Welcome to our community!".into()),
        guide_steps: vec![GuideStepDto {
            title: "Say hello".into(),
            description: "Introduce yourself in #general".into(),
            channel_id: Some("ch_general".into()),
            emoji: None,
        }],
    };

    let json = serde_json::to_string(&config).unwrap();
    let back: OnboardingConfigDto = serde_json::from_str(&json).unwrap();
    assert!(back.enabled);
    assert_eq!(back.mode, OnboardingModeDto::Guided);
    assert_eq!(back.questions.len(), 1);
    assert_eq!(back.questions[0].options.len(), 2);
    assert_eq!(back.guide_steps.len(), 1);
}

#[test]
fn onboarding_mode_serde() {
    let modes = vec![
        OnboardingModeDto::Default,
        OnboardingModeDto::Guided,
        OnboardingModeDto::Gated,
    ];
    for mode in &modes {
        let json = serde_json::to_string(mode).unwrap();
        let back: OnboardingModeDto = serde_json::from_str(&json).unwrap();
        assert_eq!(*mode, back);
    }
}

#[test]
fn welcome_screen_default() {
    let screen = WelcomeScreenDto::default();
    assert!(screen.description.is_empty());
    assert!(screen.channels.is_empty());
}

#[test]
fn welcome_screen_serde() {
    let screen = WelcomeScreenDto {
        description: "Welcome to Rekindle!".into(),
        channels: vec![
            WelcomeChannelEntryDto {
                channel_id: "ch_general".into(),
                description: "Chat with everyone".into(),
                emoji: Some("💬".into()),
            },
            WelcomeChannelEntryDto {
                channel_id: "ch_rules".into(),
                description: "Read the rules".into(),
                emoji: None,
            },
        ],
    };

    let json = serde_json::to_string(&screen).unwrap();
    let back: WelcomeScreenDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.channels.len(), 2);
    assert_eq!(back.channels[0].channel_id, "ch_general");
}
