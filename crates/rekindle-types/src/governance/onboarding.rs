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
