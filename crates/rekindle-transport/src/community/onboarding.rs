//! Onboarding and welcome screen types for manifest subkeys 10-11.

use serde::{Deserialize, Serialize};

/// Onboarding configuration (manifest subkey 10).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingConfig {
    pub enabled: bool,
    pub mode: OnboardingMode,
    #[serde(default)]
    pub default_channels: Vec<String>,
    #[serde(default)]
    pub questions: Vec<OnboardingQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome_message: Option<String>,
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
    Default,
    Guided,
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
    pub required: bool,
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
    #[serde(default)]
    pub roles_to_assign: Vec<u32>,
    #[serde(default)]
    pub channels_to_show: Vec<String>,
}

/// A guided step shown after onboarding questions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideStep {
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

/// Welcome screen (manifest subkey 11).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeScreen {
    pub description: String,
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
