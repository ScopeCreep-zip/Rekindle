//! The profile, picture and policy requests' `Debug` (plan E3.1 D4):
//! a picture and free text show their length only.

use super::super::super::CommunityRequest;

impl CommunityRequest {
    /// The profile, picture and policy requests' `Debug`.
    pub(super) fn fmt_profile(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
            other => unreachable!("{} is not a profile request", other.name()),
        }
    }
}
