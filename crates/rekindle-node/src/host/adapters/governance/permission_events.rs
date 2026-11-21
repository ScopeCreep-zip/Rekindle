//! Permission verdicts from the merged governance (plan E3.2 S9), on both
//! modes. What a member may do is a function of the merged state alone
//! (architecture §9.2, `rekindle_governance::permissions::compute_permissions`:
//! roles, assignments, channel overwrites, timeouts), the same function
//! every peer runs, so a change in it is read as the difference between the
//! state last projected and the state just merged, like membership
//! (`member_events.rs`):
//!
//! - `EffectivePermissionsChanged { member }` for each member whose base or
//!   any channel's effective permissions differ, ours included: our own
//!   windows re-read their permission flags from it (the frontend computes
//!   no policy, plan E6.11), and Discord likewise recomputes a member's
//!   permissions on every role, overwrite or timeout update;
//! - `ChannelPermissionsChanged { channel }` for each channel whose
//!   overwrite set differs (the gossip `ChannelOverwriteChanged` notice
//!   publishes none: an overwrite is a signed governance op).
//!
//! Both sides are evaluated at the same instant, so a timeout running out
//! between two merges is no transition (its end was published with it).

use rekindle_governance::permissions::compute_permissions;
use rekindle_governance::state::GovernanceState;
use rekindle_types::id::{ChannelId, PseudonymKey};

/// What changed in permissions between `prev` and `next`, at `now_secs`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PermissionTransitions {
    /// Members (pseudonym hex) whose effective permissions changed, in order.
    pub members: Vec<String>,
    /// Channels (id hex) whose overwrite set changed, in order.
    pub channels: Vec<String>,
}

impl PermissionTransitions {
    pub(super) fn is_empty(&self) -> bool {
        self.members.is_empty() && self.channels.is_empty()
    }
}

/// The permission transitions that take `prev` to `next`.
pub(super) fn permission_transitions(
    prev: &GovernanceState,
    next: &GovernanceState,
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
        .filter(|m| perms_of(prev, m) != perms_of(next, m))
        .map(PseudonymKey::to_hex)
        .collect();
    let overwrites_of = |state: &GovernanceState, channel: &ChannelId| {
        let mut set: Vec<_> = state
            .overwrites
            .iter()
            .filter(|((c, _), _)| c == channel)
            .map(|((_, target), ow)| (target.clone(), ow.target_type.clone(), ow.allow, ow.deny))
            .collect();
        set.sort();
        set
    };
    let channels = channels
        .into_iter()
        .filter(|c| overwrites_of(prev, c) != overwrites_of(next, c))
        .map(|c| hex::encode(c.0))
        .collect();
    PermissionTransitions { members, channels }
}

impl super::HostGovernance {
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
        for member in changes.members {
            self.ctx.publish_event(SubscriptionEvent::Governance(
                GovernanceEvent::EffectivePermissionsChanged {
                    community: community_id.to_string(),
                    member,
                },
            ));
        }
    }
}

#[cfg(test)]
#[path = "permission_events_tests.rs"]
mod tests;
