//! The onboarding flow and welcome screen as they cross IPC (plan E3.1 D3):
//! hex channel ids and legacy `u32` role ids, where the governance structs
//! (`crate::governance::onboarding`) carry the newtype ids the merge and the
//! codec use. The Host maps one to the other
//! (`rekindle-node`'s `host/adapters/governance/onboarding_map.rs`).

use serde::{Deserialize, Serialize};

/// A community's onboarding flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingConfigDto {
    /// Whether onboarding is enabled for new members.
    pub enabled: bool,
    /// The onboarding mode.
    pub mode: OnboardingModeDto,
    /// Channel IDs that all new members see by default.
    #[serde(default)]
    pub default_channels: Vec<String>,
    /// Questions presented during the onboarding flow.
    #[serde(default)]
    pub questions: Vec<OnboardingQuestionDto>,
    /// Optional welcome message shown at the start of onboarding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome_message: Option<String>,
    /// Guided steps shown after completing questions.
    #[serde(default)]
    pub guide_steps: Vec<GuideStepDto>,
}

impl Default for OnboardingConfigDto {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OnboardingModeDto::Default,
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
pub enum OnboardingModeDto {
    /// Standard join — assign default roles immediately.
    Default,
    /// Multi-step guided setup with questions.
    Guided,
    /// Gated — member must acknowledge rules before joining.
    Gated,
}

/// A question presented during onboarding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingQuestionDto {
    pub question_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether an answer is required to proceed.
    pub required: bool,
    /// If true, only one option can be selected; if false, multi-select.
    pub single_select: bool,
    pub options: Vec<OnboardingOptionDto>,
}

/// An option for an onboarding question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingOptionDto {
    pub option_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Role IDs to assign when this option is selected.
    #[serde(default)]
    /// Full 16-byte role ids, 32 hex characters each.
    pub roles_to_assign: Vec<String>,
    /// Channel IDs to reveal when this option is selected.
    #[serde(default)]
    pub channels_to_show: Vec<String>,
}

/// The onboarding flow an editor sends (`OnboardingConfigSet`, plan E7.15
/// S3): the view's shape, except that a new question or option carries no
/// id. The Host mints a new item's id and keeps an existing item's, and
/// refuses an id the merged flow does not hold, so ids are never the
/// caller's (backend owns policy).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingConfigEdit {
    pub enabled: bool,
    pub mode: OnboardingModeDto,
    #[serde(default)]
    pub default_channels: Vec<String>,
    #[serde(default)]
    pub questions: Vec<OnboardingQuestionEdit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome_message: Option<String>,
    #[serde(default)]
    pub guide_steps: Vec<GuideStepDto>,
}

/// A question as an editor sends it: `question_id` is `None` on a new one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingQuestionEdit {
    #[serde(default)]
    pub question_id: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub required: bool,
    pub single_select: bool,
    pub options: Vec<OnboardingOptionEdit>,
}

/// An option as an editor sends it: `option_id` is `None` on a new one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingOptionEdit {
    #[serde(default)]
    pub option_id: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Full 16-byte role ids, 32 hex characters each.
    #[serde(default)]
    pub roles_to_assign: Vec<String>,
    #[serde(default)]
    pub channels_to_show: Vec<String>,
}

impl Default for OnboardingConfigEdit {
    fn default() -> Self {
        OnboardingConfigDto::default().into()
    }
}

/// The merged flow as an edit that changes nothing: every item keeps its id.
impl From<OnboardingConfigDto> for OnboardingConfigEdit {
    fn from(c: OnboardingConfigDto) -> Self {
        Self {
            enabled: c.enabled,
            mode: c.mode,
            default_channels: c.default_channels,
            questions: c.questions.into_iter().map(Into::into).collect(),
            welcome_message: c.welcome_message,
            guide_steps: c.guide_steps,
        }
    }
}

impl From<OnboardingQuestionDto> for OnboardingQuestionEdit {
    fn from(q: OnboardingQuestionDto) -> Self {
        Self {
            question_id: Some(q.question_id),
            title: q.title,
            description: q.description,
            required: q.required,
            single_select: q.single_select,
            options: q.options.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<OnboardingOptionDto> for OnboardingOptionEdit {
    fn from(o: OnboardingOptionDto) -> Self {
        Self {
            option_id: Some(o.option_id),
            title: o.title,
            description: o.description,
            roles_to_assign: o.roles_to_assign,
            channels_to_show: o.channels_to_show,
        }
    }
}

/// A guided step shown after onboarding questions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideStepDto {
    pub title: String,
    pub description: String,
    /// Optional channel ID to highlight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Optional emoji for visual flair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

/// A community's welcome screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeScreenDto {
    /// Community description shown on the welcome screen.
    pub description: String,
    /// Up to 5 featured channels with descriptions.
    pub channels: Vec<WelcomeChannelEntryDto>,
}

/// A channel featured on the welcome screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeChannelEntryDto {
    pub channel_id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod tests;
