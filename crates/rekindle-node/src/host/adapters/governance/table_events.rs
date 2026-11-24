//! The role, channel and member-role events of a projected merge (plan
//! E3.2 S9): `GovernanceEvent::RolesChanged`, `GovernanceEvent::ChannelsChanged`
//! and `MembershipEvent::RolesChanged`, read off [`GovernanceDiff`].
//!
//! Every peer merges the same op DAG, so a change is the difference between
//! the state last projected and the state just merged (architecture §6, the
//! reader derives what it shows from merged state). Our own write reaches
//! here the same way a peer's op does (`write` installs its merge through
//! `set_governance_state`), so each change is announced once, from one
//! place, whoever made it. A merge that changes no table announces nothing.
//!
//! Each event carries the whole table from the state now in place (the
//! same builders `role list` and `channel list` answer with), so a frontend
//! replaces its copy rather than patching it.

use rekindle_governance::diff::GovernanceDiff;
use rekindle_types::display::CategoryDisplay;
use rekindle_types::subscription_events::{GovernanceEvent, MembershipEvent, SubscriptionEvent};

use super::HostGovernance;

/// Which whole tables a diff invalidates.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct TableChanges {
    /// A role was defined, edited or deleted.
    pub roles: bool,
    /// A channel or category was created, edited or deleted.
    pub channels: bool,
}

impl TableChanges {
    pub(super) fn of(diff: &GovernanceDiff) -> Self {
        Self {
            roles: !diff.roles.is_empty(),
            channels: !diff.channels.is_empty() || !diff.categories.is_empty(),
        }
    }
}

impl HostGovernance {
    /// Publish the table events of `diff`, read against the state of
    /// `community_id` now projected.
    pub(super) fn publish_table_changes(&self, community_id: &str, diff: &GovernanceDiff) {
        let changes = TableChanges::of(diff);
        if changes.roles {
            self.ctx.publish_event(SubscriptionEvent::Governance(
                GovernanceEvent::RolesChanged {
                    community: community_id.to_string(),
                    roles: crate::daemon::dispatch::governance::role_displays(
                        &self.ctx,
                        community_id,
                    ),
                },
            ));
        }
        if changes.channels {
            self.ctx.publish_event(SubscriptionEvent::Governance(
                GovernanceEvent::ChannelsChanged {
                    community: community_id.to_string(),
                    channels: crate::daemon::dispatch::channel::channel_overviews(
                        &self.ctx,
                        community_id,
                    ),
                    categories: self.category_displays(community_id),
                },
            ));
        }
        if diff.member_roles.is_empty() {
            return;
        }
        let assignments = (self.ctx.communities.governance_state(community_id))
            .map(|gov| gov.role_assignments)
            .unwrap_or_default();
        for member in &diff.member_roles {
            let mut role_ids: Vec<String> = (assignments.get(member).into_iter().flatten())
                .map(|id| id.to_hex())
                .collect();
            role_ids.sort();
            self.ctx.publish_event(SubscriptionEvent::Membership(
                MembershipEvent::RolesChanged {
                    community: community_id.to_string(),
                    pseudonym: member.to_hex(),
                    role_ids,
                },
            ));
        }
    }

    /// Categories for a community, sorted by position then id.
    ///
    /// There is no `category list` command to share a builder with, so
    /// this is the one place the projection lives; `ChannelsChanged`
    /// carries it beside the channel tree because a channel's
    /// `category_id` is meaningless to a client that has not seen the
    /// categories.
    fn category_displays(&self, community_id: &str) -> Vec<CategoryDisplay> {
        let Some(gov) = self.ctx.communities.governance_state(community_id) else {
            return Vec::new();
        };
        let mut out: Vec<CategoryDisplay> = gov
            .categories
            .iter()
            .map(|(id, category)| CategoryDisplay {
                id: hex::encode(id.0),
                name: category.name.clone(),
                sort_order: i32::try_from(category.position).unwrap_or(i32::MAX),
            })
            .collect();
        out.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.id.cmp(&b.id)));
        out
    }
}

#[cfg(test)]
#[path = "table_events_tests.rs"]
mod tests;
