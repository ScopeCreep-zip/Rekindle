//! The moderation requests' `Debug` (plan E3.1).

use super::super::super::CommunityRequest;

impl CommunityRequest {
    /// The kick, ban and timeout requests' `Debug`.
    pub(super) fn fmt_moderation(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kick {
                community,
                target_pseudonym,
            } => f
                .debug_struct("Kick")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::Ban {
                community,
                target_pseudonym,
                reason,
            } => f
                .debug_struct("Ban")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .field("reason", reason)
                .finish(),
            Self::Unban {
                community,
                target_pseudonym,
            } => f
                .debug_struct("Unban")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::Timeout {
                community,
                target_pseudonym,
                duration_seconds,
                reason,
            } => f
                .debug_struct("Timeout")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .field("duration_seconds", duration_seconds)
                .field("reason", reason)
                .finish(),
            Self::TimeoutRemove {
                community,
                target_pseudonym,
            } => f
                .debug_struct("TimeoutRemove")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::BanList { community } => f
                .debug_struct("BanList")
                .field("community", community)
                .finish(),
            other => unreachable!("{} is not a moderation request", other.name()),
        }
    }
}
