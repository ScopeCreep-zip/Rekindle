//! The member management: roles and their assignment, kicks, bans, timeouts, member blocks, automod and mod reports (plan E3.1), carried as `CommunityRequest::Members`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A member management request.
#[derive(Clone, Serialize, Deserialize)]
pub enum MemberRequest {
    /// List all roles in a community.
    RoleList { community: String },

    /// Create a new role.
    RoleCreate {
        community: String,
        name: String,
        permissions: u64,
        color: u32,
        /// Members with it are listed in their own sidebar group.
        hoist: bool,
        /// `@role` reaches everyone holding it.
        mentionable: bool,
        /// Members may take it themselves.
        self_assignable: bool,
        /// At most one role of a group per member (architecture §19.4).
        exclusion_group: Option<String>,
    },

    /// Update a role's properties.
    RoleUpdate {
        community: String,
        role_id: u32,
        name: Option<String>,
        permissions: Option<u64>,
        color: Option<u32>,
        position: Option<i32>,
        hoist: Option<bool>,
        mentionable: Option<bool>,
        self_assignable: Option<bool>,
        exclusion_group: rekindle_types::governance::ExclusionGroupEdit,
    },

    /// Delete a role.
    RoleDelete { community: String, role_id: u32 },

    /// Assign a role to a member.
    RoleAssign {
        community: String,
        member_pseudonym: String,
        role_id: u32,
    },

    /// Remove a role from a member.
    RoleUnassign {
        community: String,
        member_pseudonym: String,
        role_id: u32,
    },

    /// Take a self-assignable role ourselves.
    RoleSelfAssign { community: String, role_id: u32 },

    /// Drop a self-assignable role we hold.
    RoleSelfUnassign { community: String, role_id: u32 },

    /// Kick a member (sync gossip broadcast).
    Kick {
        community: String,
        target_pseudonym: String,
    },

    /// Ban a member (persist to governance + gossip broadcast).
    Ban {
        community: String,
        target_pseudonym: String,
        reason: Option<String>,
    },

    /// Unban a member.
    Unban {
        community: String,
        target_pseudonym: String,
    },

    /// Block a member in this community, by the pseudonym they post under
    /// there (D26): our ingest gate drops their posts. Only we see it.
    MemberBlock {
        community: String,
        target_pseudonym: String,
    },

    /// Lift a community block.
    MemberUnblock {
        community: String,
        target_pseudonym: String,
    },

    /// The pseudonyms we block in a community, newest first.
    MemberBlockList { community: String },

    /// Timeout a member for a duration.
    Timeout {
        community: String,
        target_pseudonym: String,
        duration_seconds: u64,
        reason: Option<String>,
    },

    /// Lift a member's timeout early.
    TimeoutRemove {
        community: String,
        target_pseudonym: String,
    },

    /// List all active bans in a community.
    BanList { community: String },

    /// The community's enabled automod rules.
    AutomodRules { community: String },

    /// Create a rule (no `rule_id`) or replace one; answered with its id.
    AutomodRuleSet {
        community: String,
        rule_id: Option<String>,
        name: String,
        enabled: bool,
        keywords: Vec<String>,
        regex_patterns: Vec<String>,
        action: rekindle_types::governance::AutoModAction,
    },

    /// Turn a rule off for good.
    AutomodRuleDelete { community: String, rule_id: String },

    /// The community's moderator queue (`MANAGE_MESSAGES`).
    ModReportList { community: String },
}

impl MemberRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::RoleList { .. } => ("RoleList", Query, Authenticated),
            Self::RoleCreate { .. } => ("RoleCreate", Write, Authenticated),
            Self::RoleUpdate { .. } => ("RoleUpdate", Write, Authenticated),
            Self::RoleDelete { .. } => ("RoleDelete", Write, Authenticated),
            Self::RoleAssign { .. } => ("RoleAssign", Write, Authenticated),
            Self::RoleUnassign { .. } => ("RoleUnassign", Write, Authenticated),
            Self::RoleSelfAssign { .. } => ("RoleSelfAssign", Write, Authenticated),
            Self::RoleSelfUnassign { .. } => ("RoleSelfUnassign", Write, Authenticated),
            Self::Kick { .. } => ("Kick", Write, Authenticated),
            Self::Ban { .. } => ("Ban", Write, Authenticated),
            Self::MemberBlock { .. } => ("MemberBlock", Write, Authenticated),
            Self::MemberUnblock { .. } => ("MemberUnblock", Write, Authenticated),
            Self::MemberBlockList { .. } => ("MemberBlockList", Query, Authenticated),
            Self::Unban { .. } => ("Unban", Write, Authenticated),
            Self::Timeout { .. } => ("Timeout", Write, Authenticated),
            Self::TimeoutRemove { .. } => ("TimeoutRemove", Write, Authenticated),
            Self::BanList { .. } => ("BanList", Query, Authenticated),
            Self::AutomodRules { .. } => ("AutomodRules", Query, Authenticated),
            Self::AutomodRuleSet { .. } => ("AutomodRuleSet", Write, Authenticated),
            Self::AutomodRuleDelete { .. } => ("AutomodRuleDelete", Write, Authenticated),
            Self::ModReportList { .. } => ("ModReportList", Query, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::RoleList { .. }
            | Self::BanList { .. }
            | Self::MemberBlockList { .. }
            | Self::AutomodRules { .. }
            | Self::ModReportList { .. } => C::CommunitiesRead,
            Self::MemberBlock { .. }
            | Self::MemberUnblock { .. }
            | Self::RoleCreate { .. }
            | Self::RoleUpdate { .. }
            | Self::RoleDelete { .. }
            | Self::RoleSelfAssign { .. }
            | Self::RoleSelfUnassign { .. } => C::CommunitiesWrite,
            Self::RoleAssign { .. }
            | Self::RoleUnassign { .. }
            | Self::Kick { .. }
            | Self::Ban { .. }
            | Self::Unban { .. }
            | Self::Timeout { .. }
            | Self::TimeoutRemove { .. }
            | Self::AutomodRuleSet { .. }
            | Self::AutomodRuleDelete { .. } => C::ModerationWrite,
        }
    }
}
