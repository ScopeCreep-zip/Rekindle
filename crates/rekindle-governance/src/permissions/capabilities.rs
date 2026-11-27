//! Why a member lacks each community-window capability (plan E3.2 S9),
//! from the merged governance alone: the bits that grant it are the ones
//! the validator checks on the entry the control writes
//! (`validate::validate_write`), so a capability the window enables is one
//! every reader accepts. The cause is the real one: a seated creator
//! holds everything (decision (53)); a timeout (step 7 of
//! [`super::compute_permissions`]) is named only when the member would
//! hold the capability without it; else the missing permission. The
//! channel-scoped controls (a moderator's delete, the soundboard) are
//! judged with the channel's overwrites, as the validator judges their
//! entries, and name an overwrite when it is the cause.

use rekindle_types::dto::community_capability::{ChannelCapability, CommunityCapability};
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::permissions::{
    BAN_MEMBERS, CREATE_EVENTS, CREATE_EXPRESSIONS, CREATE_INVITES, CREATE_PUBLIC_THREADS,
    KICK_MEMBERS, MANAGE_CHANNELS, MANAGE_COMMUNITY, MANAGE_EVENTS, MANAGE_EXPRESSIONS,
    MANAGE_MESSAGES, MANAGE_ROLES, SEND_IN_THREADS, TIMEOUT_MEMBERS, USE_SOUNDBOARD,
    VIEW_AUDIT_LOG, VIEW_INSIGHTS,
};

use super::{compute_permissions, holds_creator_seat};
use crate::state::GovernanceState;

/// Why a capability is not held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityDenial {
    /// No role grants the permission; `permission` is its name as the role
    /// editor shows it.
    Missing { permission: &'static str },
    /// The member would hold it, but is timed out until `until_secs`
    /// (unix seconds).
    TimedOut { until_secs: u64 },
    /// Only a seated creator holds it.
    NotCreator,
    /// The member holds the permission community-wide, but the channel's
    /// overwrites take it away there.
    ChannelDenied { permission: &'static str },
}

/// What grants `cap`: any one of `bits` (named `label`), or a creator seat.
enum Grant {
    Any { bits: u64, label: &'static str },
    CreatorSeat,
}

const fn grant(cap: CommunityCapability) -> Grant {
    use CommunityCapability as C;
    let (bits, label) = match cap {
        C::ManageCommunity | C::ManageWelcome | C::ManageAutomod | C::ManageSecurity => {
            (MANAGE_COMMUNITY, "Manage Community")
        }
        C::ManageChannels | C::SetTopic | C::ManageServers => (MANAGE_CHANNELS, "Manage Channels"),
        C::ManageRoles => (MANAGE_ROLES, "Manage Roles"),
        C::KickMembers => (KICK_MEMBERS, "Kick Members"),
        C::BanMembers => (BAN_MEMBERS, "Ban Members"),
        C::ModerateMembers => (TIMEOUT_MEMBERS, "Moderate Members"),
        C::CreateInvite => (CREATE_INVITES, "Create Invites"),
        C::ViewAuditLog => (VIEW_AUDIT_LOG, "View Audit Log"),
        C::ViewInsights => (VIEW_INSIGHTS, "View Insights"),
        C::ManageExpressions => (MANAGE_EXPRESSIONS, "Manage Expressions"),
        C::CreateExpressions => (
            CREATE_EXPRESSIONS | MANAGE_EXPRESSIONS,
            "Create Expressions or Manage Expressions",
        ),
        C::ManageEvents => (MANAGE_EVENTS, "Manage Events"),
        C::CreateEvents => (CREATE_EVENTS, "Create Events"),
        C::OfferSuccessor => return Grant::CreatorSeat,
    };
    Grant::Any { bits, label }
}

/// What a member holds in one scope (the community, or one channel with
/// its overwrites) at `now_unix`, and what it would hold there once its
/// running timeout ends.
struct Held {
    now: u64,
    timeout_end: Option<u64>,
    untimed: Option<u64>,
}

impl Held {
    fn of(
        member: &PseudonymKey,
        channel: Option<&ChannelId>,
        state: &GovernanceState,
        now_unix: u64,
    ) -> Self {
        let timeout_end = (state.timeouts.get(member))
            .filter(|t| !t.is_expired(now_unix))
            .map(|t| t.started_at.saturating_add(t.duration_seconds));
        Self {
            now: compute_permissions(member, channel, state, now_unix),
            timeout_end,
            // The same state with the timeout run out: what the member
            // holds apart from it.
            untimed: timeout_end.map(|end| compute_permissions(member, channel, state, end)),
        }
    }

    /// Why one of `bits` (named `label`) is not held; none when it is.
    fn denial(&self, bits: u64, label: &'static str) -> Option<CapabilityDenial> {
        if self.now & bits != 0 {
            return None;
        }
        Some(match (self.timeout_end, self.untimed) {
            (Some(until_secs), Some(held)) if held & bits != 0 => {
                CapabilityDenial::TimedOut { until_secs }
            }
            _ => CapabilityDenial::Missing { permission: label },
        })
    }
}

/// The capabilities `member` lacks in `state` at `now_unix`, each with its
/// cause, in [`CommunityCapability::ALL`] order; empty for a seated
/// creator.
#[must_use]
pub fn denied_capabilities(
    member: &PseudonymKey,
    state: &GovernanceState,
    now_unix: u64,
) -> Vec<(CommunityCapability, CapabilityDenial)> {
    let seated = holds_creator_seat(member, state);
    let held = Held::of(member, None, state, now_unix);
    CommunityCapability::ALL
        .into_iter()
        .filter_map(|cap| {
            let denial = match grant(cap) {
                Grant::CreatorSeat if seated => return None,
                Grant::CreatorSeat => CapabilityDenial::NotCreator,
                Grant::Any { bits, label } => held.denial(bits, label)?,
            };
            Some((cap, denial))
        })
        .collect()
}

/// The permission a channel-scoped capability needs, and its name as the
/// role editor shows it: the bit the validator checks with the channel's
/// overwrites (`AdminDelete`: `MANAGE_MESSAGES`; a `SoundboardPlay`'s
/// readers: `USE_SOUNDBOARD`).
const fn channel_grant(cap: ChannelCapability) -> (u64, &'static str) {
    match cap {
        ChannelCapability::ManageMessages => (MANAGE_MESSAGES, "Manage Messages"),
        ChannelCapability::UseSoundboard => (USE_SOUNDBOARD, "Use Soundboard"),
        ChannelCapability::CreatePublicThreads => (CREATE_PUBLIC_THREADS, "Create Public Threads"),
        ChannelCapability::SendInThreads => (SEND_IN_THREADS, "Send In Threads"),
    }
}

/// The role editor name of the permission a channel capability needs.
#[must_use]
pub const fn channel_permission_name(cap: ChannelCapability) -> &'static str {
    channel_grant(cap).1
}

/// The channel-scoped capabilities `member` lacks in `channel` at
/// `now_unix`, each with its cause, in [`ChannelCapability::ALL`] order.
/// A permission held community-wide that the channel's overwrites take
/// away is [`CapabilityDenial::ChannelDenied`].
#[must_use]
pub fn denied_in_channel(
    member: &PseudonymKey,
    channel: &ChannelId,
    state: &GovernanceState,
    now_unix: u64,
) -> Vec<(ChannelCapability, CapabilityDenial)> {
    let base = Held::of(member, None, state, now_unix);
    let here = Held::of(member, Some(channel), state, now_unix);
    ChannelCapability::ALL
        .into_iter()
        .filter_map(|cap| {
            let (bits, label) = channel_grant(cap);
            let denial = match here.denial(bits, label)? {
                // Held community-wide, now or once a timeout ends.
                CapabilityDenial::Missing { .. }
                    if (base.now | base.untimed.unwrap_or(0)) & bits != 0 =>
                {
                    CapabilityDenial::ChannelDenied { permission: label }
                }
                denial => denial,
            };
            Some((cap, denial))
        })
        .collect()
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
