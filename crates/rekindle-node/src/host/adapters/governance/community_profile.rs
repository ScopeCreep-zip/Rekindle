//! Our per-community profile and the community's policy on the Host (plan
//! E3.1 D4), both modes.
//!
//! The profile (bio, pronouns, theme color, badges, avatar and banner
//! hashes) is ours alone: it is kept on the community in the store and
//! goes out with our next presence write, which a presence tick sends now.
//! The policy is a `CommunityPolicy` governance entry: readers validate
//! `MANAGE_COMMUNITY` at merge, and the merge projects it into the store.

use rekindle_governance::state::CommunityPolicyState;
use rekindle_governance_runtime::{GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_types::dto::community::CommunityPolicyView;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::permissions;

use super::HostGovernance;

/// Our profile in one community, as the user set it.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProfileEdit {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// A new community policy.
#[derive(Debug, Clone)]
pub(crate) struct PolicyEdit {
    pub policy_text: Option<String>,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
}

impl HostGovernance {
    /// Make `edit` our profile in `community`, and publish it with a
    /// presence tick.
    ///
    /// # Errors
    /// The community is not joined.
    pub(crate) fn update_profile(&self, community: &str, edit: ProfileEdit) -> Result<(), String> {
        {
            let mut map = self.ctx.communities.map().write();
            let c = map.get_mut(community).ok_or("unknown community")?;
            c.my_bio = edit.bio;
            c.my_pronouns = edit.pronouns;
            c.my_theme_color = edit.theme_color;
            c.my_badges = edit.badges;
            c.my_avatar_ref = edit.avatar_ref;
            c.my_banner_ref = edit.banner_ref;
        }
        self.spawn_presence_poll_tick_impl(community);
        Ok(())
    }

    /// `community`'s merged policy; the defaults before any merged.
    ///
    /// # Errors
    /// We lack `VIEW_CHANNELS`.
    pub(crate) fn community_policy(
        &self,
        community: &str,
    ) -> Result<CommunityPolicyView, GovernanceRuntimeError> {
        self.require_permission(community, permissions::VIEW_CHANNELS)?;
        let policy = self
            .governance_state_impl(community)
            .and_then(|s| s.community_policy);
        Ok(policy.map_or(
            CommunityPolicyView {
                policy_text: None,
                max_joins_per_interval: CommunityPolicyState::DEFAULT_MAX_JOINS_PER_INTERVAL,
                join_interval_seconds: CommunityPolicyState::DEFAULT_JOIN_INTERVAL_SECONDS,
            },
            |p| CommunityPolicyView {
                policy_text: p.policy_text,
                max_joins_per_interval: p.max_joins_per_interval,
                join_interval_seconds: p.join_interval_seconds,
            },
        ))
    }

    /// Write `edit` as `community`'s policy.
    ///
    /// # Errors
    /// We lack `MANAGE_COMMUNITY`, or the entry could not be written.
    pub(crate) async fn set_community_policy(
        &self,
        community: &str,
        edit: PolicyEdit,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community, permissions::MANAGE_COMMUNITY)?;
        let lamport = self.next_governance_lamport(community)?;
        rekindle_governance_runtime::apply::write_entry(
            self,
            community,
            GovernanceEntry::CommunityPolicy {
                policy_text: edit.policy_text,
                max_joins_per_interval: edit.max_joins_per_interval,
                join_interval_seconds: edit.join_interval_seconds,
                lamport,
            },
        )
        .await
    }
}
