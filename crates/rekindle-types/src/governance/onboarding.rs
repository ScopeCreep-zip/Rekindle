//! Onboarding-flow and welcome-screen structs referenced by
//! [`super::entry::GovernanceEntry::OnboardingConfig`] and
//! [`super::entry::GovernanceEntry::WelcomeScreen`].

use serde::{Deserialize, Serialize};

use crate::id::{ChannelId, RoleId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OnboardingQuestion {
    pub question_id: String,
    pub title: String,
    pub description: Option<String>,
    pub required: bool,
    pub single_select: bool,
    pub options: Vec<OnboardingOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OnboardingOption {
    pub option_id: String,
    pub title: String,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub roles_to_assign: Vec<RoleId>,
    pub channels_to_show: Vec<ChannelId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuideStep {
    pub title: String,
    pub description: String,
    pub channel_id: Option<ChannelId>,
    pub emoji: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WelcomeChannel {
    pub channel_id: ChannelId,
    pub description: String,
    pub emoji: Option<String>,
}

// The bounds an onboarding flow and a welcome screen respect, whoever wrote
// them (architecture §19.1, §19.3); `rekindle_governance::validate::bounds`
// applies them at the merge, at the write and at the IPC boundary.

/// The most questions an onboarding flow asks.
pub const MAX_ONBOARDING_QUESTIONS: usize = 5;
/// The most options one question offers.
pub const MAX_ONBOARDING_OPTIONS_PER_QUESTION: usize = 10;
/// The most guide steps after the questions.
pub const MAX_ONBOARDING_GUIDE_STEPS: usize = 10;
/// The longest welcome message, in chars.
pub const MAX_ONBOARDING_WELCOME_CHARS: usize = 500;
/// The longest question, option or guide-step title, in chars.
pub const MAX_ONBOARDING_TITLE_CHARS: usize = 100;
/// The longest question or option id, in chars.
pub const MAX_ONBOARDING_ID_CHARS: usize = 64;
/// The most channels a welcome screen features.
pub const MAX_WELCOME_SCREEN_CHANNELS: usize = 5;
