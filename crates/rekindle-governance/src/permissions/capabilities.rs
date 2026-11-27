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
pub use rekindle_types::permissions::PERMISSION_NAMES;
use rekindle_types::permissions::{
    self, BAN_MEMBERS, CREATE_EVENTS, CREATE_EXPRESSIONS, CREATE_INVITES, CREATE_PUBLIC_THREADS,
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
    TimedOut {
        until_secs: u64,
        permission: &'static str,
    },
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
            (MANAGE_COMMUNITY, permission_name(MANAGE_COMMUNITY))
        }
        C::ManageChannels | C::SetTopic | C::ManageServers => {
            (MANAGE_CHANNELS, permission_name(MANAGE_CHANNELS))
        }
        C::ManageRoles => (MANAGE_ROLES, permission_name(MANAGE_ROLES)),
        C::KickMembers => (KICK_MEMBERS, permission_name(KICK_MEMBERS)),
        C::BanMembers => (BAN_MEMBERS, permission_name(BAN_MEMBERS)),
        C::ModerateMembers => (TIMEOUT_MEMBERS, permission_name(TIMEOUT_MEMBERS)),
        C::CreateInvite => (CREATE_INVITES, permission_name(CREATE_INVITES)),
        C::ViewAuditLog => (VIEW_AUDIT_LOG, permission_name(VIEW_AUDIT_LOG)),
        C::ViewInsights => (VIEW_INSIGHTS, permission_name(VIEW_INSIGHTS)),
        C::ManageExpressions => (MANAGE_EXPRESSIONS, permission_name(MANAGE_EXPRESSIONS)),
        C::CreateExpressions => (
            CREATE_EXPRESSIONS | MANAGE_EXPRESSIONS,
            "Create Expressions or Manage Expressions",
        ),
        C::ManageEvents => (MANAGE_EVENTS, permission_name(MANAGE_EVENTS)),
        C::CreateEvents => (CREATE_EVENTS, permission_name(CREATE_EVENTS)),
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
            (Some(until_secs), Some(held)) if held & bits != 0 => CapabilityDenial::TimedOut {
                until_secs,
                permission: label,
            },
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

const fn permission_name(bit: u64) -> &'static str {
    let mut index = 0;
    while index < PERMISSION_NAMES.len() {
        let (named_bit, name) = PERMISSION_NAMES[index];
        if named_bit == bit {
            return name;
        }
        index += 1;
    }
    panic!("permission bit must have a name")
}

/// The permission a channel capability needs.
#[must_use]
pub const fn channel_permission_bit(cap: ChannelCapability) -> u64 {
    match cap {
        ChannelCapability::ManageMessages => MANAGE_MESSAGES,
        ChannelCapability::UseSoundboard => USE_SOUNDBOARD,
        ChannelCapability::CreatePublicThreads => CREATE_PUBLIC_THREADS,
        ChannelCapability::SendInThreads => SEND_IN_THREADS,
    }
}

/// The role editor name of the permission a channel capability needs.
#[must_use]
pub const fn channel_permission_name(cap: ChannelCapability) -> &'static str {
    permission_name(channel_permission_bit(cap))
}

/// One CAPR cause per missing required bit, or `None` if a requirement
/// cannot be named.
#[must_use]
pub fn denied_permissions(
    member: &PseudonymKey,
    channel: Option<&ChannelId>,
    state: &GovernanceState,
    now_unix: u64,
    required: u64,
) -> Option<Vec<CapabilityDenial>> {
    if required & !permissions::DEFINED != 0 {
        return None;
    }
    let here = Held::of(member, channel, state, now_unix);
    let missing = required & !here.now;
    if missing == 0 {
        return Some(Vec::new());
    }
    let base = channel.map(|_| Held::of(member, None, state, now_unix));
    let denials: Vec<_> = PERMISSION_NAMES
        .iter()
        .filter(|(bit, _)| missing & bit != 0)
        .filter_map(|&(bit, name)| denial_in_scope(&here, base.as_ref(), bit, name))
        .collect();
    (denials.len() == missing.count_ones() as usize).then_some(denials)
}

/// The channel-scoped capabilities `member` lacks in `channel` at
/// `now_unix`, each with its cause, in [`ChannelCapability::ALL`] order.
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
            let denial = denial_in_scope(
                &here,
                Some(&base),
                channel_permission_bit(cap),
                channel_permission_name(cap),
            )?;
            Some((cap, denial))
        })
        .collect()
}

fn denial_in_scope(
    here: &Held,
    base: Option<&Held>,
    bit: u64,
    name: &'static str,
) -> Option<CapabilityDenial> {
    let denial = match here.denial(bit, name)? {
        // Held community-wide, now or once a timeout ends.
        CapabilityDenial::Missing { .. }
            if base.is_some_and(|base| (base.now | base.untimed.unwrap_or(0)) & bit != 0) =>
        {
            CapabilityDenial::ChannelDenied { permission: name }
        }
        denial => denial,
    };
    Some(denial)
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
