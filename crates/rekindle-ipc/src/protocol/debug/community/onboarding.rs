//! The onboarding and welcome-screen requests' `Debug` (plan E3.1 D3): the
//! answers say what a member chose, so only their count is shown.

use super::super::super::CommunityRequest;

impl CommunityRequest {
    /// The onboarding and welcome-screen requests' `Debug`.
    pub(super) fn fmt_onboarding(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OnboardingConfigGet { community } => f
                .debug_struct("OnboardingConfigGet")
                .field("community", community)
                .finish(),
            Self::OnboardingConfigSet { community, config } => f
                .debug_struct("OnboardingConfigSet")
                .field("community", community)
                .field("questions", &config.questions.len())
                .finish_non_exhaustive(),
            Self::WelcomeScreenGet { community } => f
                .debug_struct("WelcomeScreenGet")
                .field("community", community)
                .finish(),
            Self::WelcomeScreenSet { community, screen } => f
                .debug_struct("WelcomeScreenSet")
                .field("community", community)
                .field("channels", &screen.channels.len())
                .finish_non_exhaustive(),
            Self::OnboardingSubmit {
                community,
                answers,
                acknowledged_rules,
            } => f
                .debug_struct("OnboardingSubmit")
                .field("community", community)
                .field("answers", &answers.len())
                .field("acknowledged_rules", acknowledged_rules)
                .finish(),
            Self::OnboardingComplete { community } => f
                .debug_struct("OnboardingComplete")
                .field("community", community)
                .finish(),
            other => unreachable!("{} is not an onboarding request", other.name()),
        }
    }
}
