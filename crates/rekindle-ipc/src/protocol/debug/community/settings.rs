//! `Debug` for the community settings requests: free text shows its length only.

use crate::protocol::SettingsRequest;

impl std::fmt::Debug for SettingsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            Self::CommunitySetAvatar {
                community,
                image,
                kind,
            }
            | Self::CommunitySetBanner {
                community,
                image,
                kind,
            } => f
                .debug_struct(if matches!(self, Self::CommunitySetAvatar { .. }) {
                    "CommunitySetAvatar"
                } else {
                    "CommunitySetBanner"
                })
                .field("community", community)
                .field("image_len", &image.len())
                .field("kind", kind)
                .finish(),
            Self::CommunityUpdateProfile {
                community,
                bio,
                pronouns,
                theme_color,
                badges,
                avatar_ref,
                banner_ref,
            } => f
                .debug_struct("CommunityUpdateProfile")
                .field("community", community)
                .field("bio_len", &bio.as_ref().map(String::len))
                .field("pronouns_len", &pronouns.as_ref().map(String::len))
                .field("theme_color", theme_color)
                .field("badges", &badges.len())
                .field("avatar_ref", avatar_ref)
                .field("banner_ref", banner_ref)
                .finish(),
            Self::CommunityAvatarGet { community, hash } => f
                .debug_struct("CommunityAvatarGet")
                .field("community", community)
                .field("hash", hash)
                .finish(),
            Self::CommunityPolicyGet { community } => f
                .debug_struct("CommunityPolicyGet")
                .field("community", community)
                .finish(),
            Self::CommunityPolicySet {
                community,
                policy_text,
                max_joins_per_interval,
                join_interval_seconds,
            } => f
                .debug_struct("CommunityPolicySet")
                .field("community", community)
                .field("policy_text_len", &policy_text.as_ref().map(String::len))
                .field("max_joins_per_interval", max_joins_per_interval)
                .field("join_interval_seconds", join_interval_seconds)
                .finish(),
        }
    }
}
