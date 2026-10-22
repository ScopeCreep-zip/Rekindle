//! The community settings: our per-community profile and pictures, the policy, onboarding and the welcome screen (plan E3.1), carried as `CommunityRequest::Settings`.

use rekindle_types::dto::onboarding::{OnboardingConfigDto, WelcomeScreenDto};
use rekindle_types::presence::OnboardingAnswer;
use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A community settings request.
#[derive(Clone, Serialize, Deserialize)]
pub enum SettingsRequest {
    /// Our per-community profile: bio, pronouns, theme color, badges and
    /// the avatar and banner hashes. Published with our presence.
    CommunityUpdateProfile {
        community: String,
        bio: Option<String>,
        pronouns: Option<String>,
        theme_color: Option<u32>,
        badges: Vec<String>,
        avatar_ref: Option<String>,
        banner_ref: Option<String>,
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
        config: OnboardingConfigDto,
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
            | Self::CommunityPolicyGet { .. } => C::CommunitiesRead,
            Self::CommunityUpdateProfile { .. }
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
