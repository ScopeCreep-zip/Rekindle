//! Why a member lacks each community-window capability (plan E3.2 S9),
//! from the merged governance alone: the bits that grant it are the ones
//! the validator checks on the entry the control writes
//! (`validate::validate_write`), so a capability the window enables is one
//! every reader accepts. The cause is the real one: a seated creator
//! holds everything (decision (53)); a timeout (step 7 of
//! [`super::compute_permissions`]) is named only when the member would
//! hold the capability without it; else the missing permission.

use rekindle_types::dto::community_capability::CommunityCapability;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{
    BAN_MEMBERS, CREATE_EVENTS, CREATE_EXPRESSIONS, CREATE_INVITES, KICK_MEMBERS, MANAGE_CHANNELS,
    MANAGE_COMMUNITY, MANAGE_EVENTS, MANAGE_EXPRESSIONS, MANAGE_ROLES, TIMEOUT_MEMBERS,
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
    let now = compute_permissions(member, None, state, now_unix);
    let timeout_end = (state.timeouts.get(member))
        .filter(|t| !t.is_expired(now_unix))
        .map(|t| t.started_at.saturating_add(t.duration_seconds));
    // The same state with the timeout run out: what the member holds apart
    // from it.
    let untimed = timeout_end.map(|end| compute_permissions(member, None, state, end));
    CommunityCapability::ALL
        .into_iter()
        .filter_map(|cap| {
            let denial = match grant(cap) {
                Grant::CreatorSeat if seated => return None,
                Grant::CreatorSeat => CapabilityDenial::NotCreator,
                Grant::Any { bits, .. } if now & bits != 0 => return None,
                Grant::Any { bits, label } => match (timeout_end, untimed) {
                    (Some(until_secs), Some(held)) if held & bits != 0 => {
                        CapabilityDenial::TimedOut { until_secs }
                    }
                    _ => CapabilityDenial::Missing { permission: label },
                },
            };
            Some((cap, denial))
        })
        .collect()
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
