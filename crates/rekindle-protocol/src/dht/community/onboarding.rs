//! Onboarding and welcome screen types for manifest subkeys 10-11.
//!
//! - Subkey 10: `OnboardingConfig` — join flow settings, questions, guide steps
//! - Subkey 11: `WelcomeScreen` — featured channels and description

use serde::{Deserialize, Serialize};

/// Onboarding configuration stored in manifest subkey 10.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingConfig {
    /// Whether onboarding is enabled for new members.
    pub enabled: bool,
    /// The onboarding mode.
    pub mode: OnboardingMode,
    /// Channel IDs that all new members see by default.
    #[serde(default)]
    pub default_channels: Vec<String>,
    /// Questions presented during the onboarding flow.
    #[serde(default)]
    pub questions: Vec<OnboardingQuestion>,
    /// Optional welcome message shown at the start of onboarding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome_message: Option<String>,
    /// Guided steps shown after completing questions.
    #[serde(default)]
    pub guide_steps: Vec<GuideStep>,
}

impl Default for OnboardingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OnboardingMode::Default,
            default_channels: Vec::new(),
            questions: Vec::new(),
            welcome_message: None,
            guide_steps: Vec::new(),
        }
    }
}

/// Onboarding flow modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnboardingMode {
    /// Standard join — assign default roles immediately.
    Default,
    /// Multi-step guided setup with questions.
    Guided,
    /// Gated — member must acknowledge rules before joining.
    Gated,
}

/// A question presented during onboarding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingQuestion {
    pub question_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether an answer is required to proceed.
    pub required: bool,
    /// If true, only one option can be selected; if false, multi-select.
    pub single_select: bool,
    pub options: Vec<OnboardingOption>,
}

/// An option for an onboarding question.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingOption {
    pub option_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Role IDs to assign when this option is selected.
    #[serde(default)]
    pub roles_to_assign: Vec<u32>,
    /// Channel IDs to reveal when this option is selected.
    #[serde(default)]
    pub channels_to_show: Vec<String>,
}

/// A guided step shown after onboarding questions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideStep {
    pub title: String,
    pub description: String,
    /// Optional channel ID to highlight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Optional emoji for visual flair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

/// Welcome screen stored in manifest subkey 11.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeScreen {
    /// Community description shown on the welcome screen.
    pub description: String,
    /// Up to 5 featured channels with descriptions.
    pub channels: Vec<WelcomeChannelEntry>,
}

/// A channel featured on the welcome screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeChannelEntry {
    pub channel_id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_config_default() {
        let config = OnboardingConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.mode, OnboardingMode::Default);
        assert!(config.questions.is_empty());
    }

    #[test]
    fn onboarding_config_serde() {
        let config = OnboardingConfig {
            enabled: true,
            mode: OnboardingMode::Guided,
            default_channels: vec!["ch_general".into(), "ch_rules".into()],
            questions: vec![OnboardingQuestion {
                question_id: "q_01".into(),
                title: "What are you interested in?".into(),
                description: Some("Select your interests".into()),
                required: true,
                single_select: false,
                options: vec![
                    OnboardingOption {
                        option_id: "opt_gaming".into(),
                        title: "Gaming".into(),
                        description: None,
                        roles_to_assign: vec![5],
                        channels_to_show: vec!["ch_gaming".into()],
                    },
                    OnboardingOption {
                        option_id: "opt_dev".into(),
                        title: "Development".into(),
                        description: Some("Code and tech".into()),
                        roles_to_assign: vec![6],
                        channels_to_show: vec!["ch_dev".into()],
                    },
                ],
            }],
            welcome_message: Some("Welcome to our community!".into()),
            guide_steps: vec![GuideStep {
                title: "Say hello".into(),
                description: "Introduce yourself in #general".into(),
                channel_id: Some("ch_general".into()),
                emoji: None,
            }],
        };

        let json = serde_json::to_string(&config).unwrap();
        let back: OnboardingConfig = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.mode, OnboardingMode::Guided);
        assert_eq!(back.questions.len(), 1);
        assert_eq!(back.questions[0].options.len(), 2);
        assert_eq!(back.guide_steps.len(), 1);
    }

    #[test]
    fn onboarding_mode_serde() {
        let modes = vec![
            OnboardingMode::Default,
            OnboardingMode::Guided,
            OnboardingMode::Gated,
        ];
        for mode in &modes {
            let json = serde_json::to_string(mode).unwrap();
            let back: OnboardingMode = serde_json::from_str(&json).unwrap();
            assert_eq!(*mode, back);
        }
    }

    #[test]
    fn welcome_screen_default() {
        let screen = WelcomeScreen::default();
        assert!(screen.description.is_empty());
        assert!(screen.channels.is_empty());
    }

    #[test]
    fn welcome_screen_serde() {
        let screen = WelcomeScreen {
            description: "Welcome to Rekindle!".into(),
            channels: vec![
                WelcomeChannelEntry {
                    channel_id: "ch_general".into(),
                    description: "Chat with everyone".into(),
                    emoji: Some("💬".into()),
                },
                WelcomeChannelEntry {
                    channel_id: "ch_rules".into(),
                    description: "Read the rules".into(),
                    emoji: None,
                },
            ],
        };

        let json = serde_json::to_string(&screen).unwrap();
        let back: WelcomeScreen = serde_json::from_str(&json).unwrap();
        assert_eq!(back.channels.len(), 2);
        assert_eq!(back.channels[0].channel_id, "ch_general");
    }
}
