//! Permission verdicts from the merged governance (plan E3.2 S9), on both
//! modes. What a member may do is a function of the merged state alone
//! (architecture §9.2, `rekindle_governance::permissions::compute_permissions`:
//! roles, assignments, channel overwrites, timeouts), the same function
//! every peer runs, so a change in it is read as the difference between the
//! state last projected and the state just merged, like membership
//! (`member_events.rs`):
//!
//! - `EffectivePermissionsChanged { member, denied }` for each member whose base or
//!   any channel's effective permissions differ, ours included: our own
//!   windows re-read their permission flags from it (the frontend computes
//!   no policy, plan E6.11), and Discord likewise recomputes a member's
//!   permissions on every role, overwrite or timeout update; also for each
//!   member whose creator seat (`holds_creator_seat`, plan E3.2 S7s)
//!   differs, as a removed co-creator, so its view's `weAreCreator` is
//!   re-read;
//! - `ChannelPermissionsChanged { channel }` for each channel whose
//!   overwrite set differs (the gossip `ChannelOverwriteChanged` notice
//!   publishes none: an overwrite is a signed governance op).
//!
//! Both sides are evaluated at the same instant, so a timeout running out
//! between two merges is no transition (its end was published with it).

use rekindle_governance::diff::GovernanceDiff;
use rekindle_governance::permissions::{compute_permissions, holds_creator_seat};
use rekindle_governance::state::GovernanceState;
use rekindle_types::dto::community_capability::DeniedCapabilities;
use rekindle_types::id::{ChannelId, PseudonymKey};

use super::capability_words::denied_of;

/// What changed in permissions between `prev` and `next`, at `now_secs`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PermissionTransitions {
    /// Members (pseudonym hex, in order) whose effective permissions
    /// changed, each with its denied capabilities under `next`, worded
    /// (`capability_words`): what its `EffectivePermissionsChanged` carries.
    pub members: std::collections::BTreeMap<String, DeniedCapabilities>,
    /// Channels (id hex) whose overwrite set changed, in order.
    pub channels: Vec<String>,
}

impl PermissionTransitions {
    pub(super) fn is_empty(&self) -> bool {
        self.members.is_empty() && self.channels.is_empty()
    }
}

/// The permission transitions that take `prev` to `next`; `diff` is
/// `prev.diff(next)`, whose overwrite changes are the channel ones.
pub(super) fn permission_transitions(
    prev: &GovernanceState,
    next: &GovernanceState,
    diff: &GovernanceDiff,
    now_secs: u64,
) -> PermissionTransitions {
    let mut channels: Vec<&ChannelId> = next.channels.keys().chain(prev.channels.keys()).collect();
    channels.sort_by_key(|c| c.0);
    channels.dedup();
    let mut members: Vec<&PseudonymKey> = next
        .members
        .keys()
        .chain(prev.members.keys())
        .chain(next.creators.iter())
        .collect();
    members.sort_by_key(|p| p.0);
    members.dedup();
    let perms_of = |state: &GovernanceState, member: &PseudonymKey| -> Vec<u64> {
        std::iter::once(compute_permissions(member, None, state, now_secs))
            .chain(
                channels
                    .iter()
                    .map(|c| compute_permissions(member, Some(c), state, now_secs)),
            )
            .collect()
    };
    let members = members
        .into_iter()
        .filter(|m| {
            perms_of(prev, m) != perms_of(next, m)
                || holds_creator_seat(m, prev) != holds_creator_seat(m, next)
        })
        .map(|m| (m.to_hex(), denied_of(m, next, now_secs)))
        .collect();
    let channels = (diff.channel_overwrites.iter())
        .map(|c| hex::encode(c.0))
        .collect();
    PermissionTransitions { members, channels }
}

/// The permission transitions of a community's first projection (`prev`
/// absent: a join's first merge, or the unlock's load): ours alone, with
/// our denied capabilities under `next`. Before it our view denied every
/// control (`views::my_denied`, the rules not loaded), so a window opened
/// in between re-reads its flags from this; another member's verdicts were
/// never shown, so none is named.
pub(super) fn first_permission_transitions(
    next: &GovernanceState,
    me: Option<&PseudonymKey>,
    now_secs: u64,
) -> PermissionTransitions {
    let members = me
        .map(|me| (me.to_hex(), denied_of(me, next, now_secs)))
        .into_iter()
        .collect();
    PermissionTransitions {
        members,
        channels: Vec::new(),
    }
}

impl super::HostGovernance {
    /// Publish `BansChanged` for `community_id`: the merge changed its ban
    /// list (`GovernanceDiff::bans_changed`), so a bans view re-reads it.
    pub(super) fn publish_bans_changed(&self, community_id: &str) {
        use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};
        self.ctx.publish_event(SubscriptionEvent::Governance(
            GovernanceEvent::BansChanged {
                community: community_id.to_string(),
            },
        ));
    }

    /// Publish `changes` (from [`permission_transitions`]) of `community_id`.
    pub(super) fn publish_permission_transitions(
        &self,
        community_id: &str,
        changes: PermissionTransitions,
    ) {
        use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};
        for channel in changes.channels {
            self.ctx.publish_event(SubscriptionEvent::Governance(
                GovernanceEvent::ChannelPermissionsChanged {
                    community: community_id.to_string(),
                    channel,
                },
            ));
        }
        for (member, denied) in changes.members {
            self.ctx.publish_event(SubscriptionEvent::Governance(
                GovernanceEvent::EffectivePermissionsChanged {
                    community: community_id.to_string(),
                    member,
                    denied,
                },
            ));
        }
    }
}

#[cfg(test)]
#[path = "permission_events_tests.rs"]
mod tests;
