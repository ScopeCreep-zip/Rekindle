//! The community settings: our per-community profile and pictures, the policy, onboarding and the welcome screen (plan E3.1), carried as `CommunityRequest::Settings`.

use rekindle_types::dto::community_profile::{CommunityProfilePatch, ProfileField};
use rekindle_types::dto::onboarding::{OnboardingConfigEdit, WelcomeScreenDto};
use rekindle_types::presence::{OnboardingAnswer, PresencePolicyField};
use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A community settings request.
#[derive(Clone, Serialize, Deserialize)]
pub enum SettingsRequest {
    /// Edit our own community profile. Each field can stay, clear, or be set.
    CommunityUpdateProfile {
        community: String,
        patch: CommunityProfilePatch,
    },

    /// Our name on the community card. Clearing restores the short pseudonym
    /// fingerprint.
    CommunitySetDisplayName {
        community: String,
        display_name: ProfileField<String>,
    },

    /// Our presence sharing consent in one community (local only, never
    /// published; architecture §10 / §16.3): what our beat and card share.
    CommunitySetPresencePolicy {
        community: String,
        policy: rekindle_types::presence::PresenceSharingPolicy,
    },

    /// Cache `image`, declared to be `kind`, as a community avatar (a WebP
    /// of at most 128 px a side); answered with its BLAKE3 hex hash.
    CommunitySetAvatar {
        community: String,
        image: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
    },

    /// The same for a banner (at most 600 × 200 px).
    CommunitySetBanner {
        community: String,
        image: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
    },

    /// A cached avatar or banner by its hash, as a `data:image/webp` URL;
    /// `None` when it is not cached.
    CommunityAvatarGet { community: String, hash: String },

    /// The community's policy text and raid thresholds.
    CommunityPolicyGet { community: String },

    /// Write a new community policy (`MANAGE_COMMUNITY`).
    CommunityPolicySet {
        community: String,
        policy_text: Option<String>,
        max_joins_per_interval: u32,
        join_interval_seconds: u32,
    },

    /// A community's onboarding flow (the empty default when none is set).
    OnboardingConfigGet { community: String },

    /// Set a community's onboarding flow (`MANAGE_COMMUNITY`).
    OnboardingConfigSet {
        community: String,
        config: OnboardingConfigEdit,
    },

    /// A community's welcome screen (the empty default when none is set).
    WelcomeScreenGet { community: String },

    /// Set a community's welcome screen (`MANAGE_COMMUNITY`).
    WelcomeScreenSet {
        community: String,
        screen: WelcomeScreenDto,
    },

    /// Answer a community's onboarding questions; a gated community needs
    /// `acknowledged_rules`.
    OnboardingSubmit {
        community: String,
        answers: Vec<OnboardingAnswer>,
        acknowledged_rules: bool,
    },

    /// Our onboarding of a community is done.
    OnboardingComplete { community: String },

    /// Read our presence sharing consent in a joined community.
    CommunityGetPresencePolicy { community: String },

    /// Change one presence consent field, keeping the other fields as stored.
    CommunitySetPresencePolicyField {
        community: String,
        field: PresencePolicyField,
    },
}

impl SettingsRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::CommunityUpdateProfile { .. } => ("CommunityUpdateProfile", Write, Authenticated),
            Self::CommunitySetDisplayName { .. } => {
                ("CommunitySetDisplayName", Write, Authenticated)
            }
            Self::CommunitySetPresencePolicy { .. } => {
                ("CommunitySetPresencePolicy", Write, Authenticated)
            }
            Self::CommunityGetPresencePolicy { .. } => {
                ("CommunityGetPresencePolicy", Query, Authenticated)
            }
            Self::CommunitySetPresencePolicyField { .. } => {
                ("CommunitySetPresencePolicyField", Write, Authenticated)
            }
            Self::CommunitySetAvatar { .. } => ("CommunitySetAvatar", Write, Authenticated),
            Self::CommunitySetBanner { .. } => ("CommunitySetBanner", Write, Authenticated),
            Self::CommunityAvatarGet { .. } => ("CommunityAvatarGet", Query, Authenticated),
            Self::CommunityPolicyGet { .. } => ("CommunityPolicyGet", Query, Authenticated),
            Self::CommunityPolicySet { .. } => ("CommunityPolicySet", Write, Authenticated),
            Self::OnboardingConfigGet { .. } => ("OnboardingConfigGet", Query, Authenticated),
            Self::OnboardingConfigSet { .. } => ("OnboardingConfigSet", Write, Authenticated),
            Self::WelcomeScreenGet { .. } => ("WelcomeScreenGet", Query, Authenticated),
            Self::WelcomeScreenSet { .. } => ("WelcomeScreenSet", Write, Authenticated),
            Self::OnboardingSubmit { .. } => ("OnboardingSubmit", Write, Authenticated),
            Self::OnboardingComplete { .. } => ("OnboardingComplete", Write, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::OnboardingConfigGet { .. }
            | Self::WelcomeScreenGet { .. }
            | Self::CommunityAvatarGet { .. }
            | Self::CommunityGetPresencePolicy { .. }
            | Self::CommunityPolicyGet { .. } => C::CommunitiesRead,
            Self::CommunityUpdateProfile { .. }
            | Self::CommunitySetDisplayName { .. }
            | Self::CommunitySetPresencePolicyField { .. }
            | Self::CommunitySetPresencePolicy { .. }
            | Self::CommunitySetAvatar { .. }
            | Self::CommunitySetBanner { .. }
            | Self::CommunityPolicySet { .. }
            | Self::OnboardingConfigSet { .. }
            | Self::WelcomeScreenSet { .. }
            | Self::OnboardingSubmit { .. }
            | Self::OnboardingComplete { .. } => C::CommunitiesWrite,
        }
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
