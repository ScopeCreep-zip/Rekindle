//! CAPR channel capability denials derived from merged governance.

use rekindle_types::dto::community_capability::ChannelCapability;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::permissions::{
    CREATE_PUBLIC_THREADS, MANAGE_MESSAGES, SEND_IN_THREADS, USE_SOUNDBOARD,
};

use super::compute_permissions;
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
    /// The member holds the permission community-wide, but the channel's
    /// overwrites take it away there.
    ChannelDenied { permission: &'static str },
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
    let held = ChannelPermissions::of(member, channel, state, now_unix);
    ChannelCapability::ALL
        .into_iter()
        .filter_map(|cap| Some((cap, held.denial(cap)?)))
        .collect()
}

/// The cause for one channel control, using the same decision as the served map.
#[must_use]
pub fn channel_denial(
    capability: ChannelCapability,
    member: &PseudonymKey,
    channel: &ChannelId,
    state: &GovernanceState,
    now_unix: u64,
) -> Option<CapabilityDenial> {
    ChannelPermissions::of(member, channel, state, now_unix).denial(capability)
}

/// The role editor's name for the required permission.
#[must_use]
pub fn channel_permission_name(capability: ChannelCapability) -> &'static str {
    channel_grant(capability).1
}

struct ChannelPermissions {
    base: Held,
    here: Held,
}

impl ChannelPermissions {
    fn of(member: &PseudonymKey, channel: &ChannelId, state: &GovernanceState, now: u64) -> Self {
        Self {
            base: Held::of(member, None, state, now),
            here: Held::of(member, Some(channel), state, now),
        }
    }

    fn denial(&self, cap: ChannelCapability) -> Option<CapabilityDenial> {
        let (bits, label) = channel_grant(cap);
        Some(match self.here.denial(bits, label)? {
            CapabilityDenial::Missing { .. }
                if (self.base.now | self.base.untimed.unwrap_or(0)) & bits != 0 =>
            {
                CapabilityDenial::ChannelDenied { permission: label }
            }
            denial => denial,
        })
    }
}
