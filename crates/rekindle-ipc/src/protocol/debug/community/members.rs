//! `Debug` for the member management requests: free text shows its length only.

use crate::protocol::MemberRequest;

impl std::fmt::Debug for MemberRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            Self::AutomodRules { community } => f
                .debug_struct("AutomodRules")
                .field("community", community)
                .finish(),
            // The rule's words are what it was written to catch: never logged.
            Self::AutomodRuleSet {
                community,
                rule_id,
                enabled,
                keywords,
                regex_patterns,
                action,
                ..
            } => f
                .debug_struct("AutomodRuleSet")
                .field("community", community)
                .field("rule_id", rule_id)
                .field("enabled", enabled)
                .field("keywords", &keywords.len())
                .field("regex_patterns", &regex_patterns.len())
                .field("action", action)
                .finish_non_exhaustive(),
            Self::MemberBlock {
                community,
                target_pseudonym,
            } => f
                .debug_struct("MemberBlock")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::MemberUnblock {
                community,
                target_pseudonym,
            } => f
                .debug_struct("MemberUnblock")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::MemberBlockList { community } => f
                .debug_struct("MemberBlockList")
                .field("community", community)
                .finish(),
            Self::AutomodRuleDelete { community, rule_id } => f
                .debug_struct("AutomodRuleDelete")
                .field("community", community)
                .field("rule_id", rule_id)
                .finish(),
            Self::ModReportList { community } => f
                .debug_struct("ModReportList")
                .field("community", community)
                .finish(),
            Self::RoleList { community } => f
                .debug_struct("RoleList")
                .field("community", community)
                .finish(),
            Self::RoleCreate {
                community,
                name,
                permissions,
                color,
                hoist,
                mentionable,
                self_assignable,
                exclusion_group,
            } => f
                .debug_struct("RoleCreate")
                .field("community", community)
                .field("name", name)
                .field("permissions", permissions)
                .field("color", color)
                .field("hoist", hoist)
                .field("mentionable", mentionable)
                .field("self_assignable", self_assignable)
                .field("exclusion_group", exclusion_group)
                .finish(),
            Self::RoleUpdate {
                community,
                role_id,
                name,
                permissions,
                color,
                position,
                hoist,
                mentionable,
                self_assignable,
                exclusion_group,
            } => f
                .debug_struct("RoleUpdate")
                .field("community", community)
                .field("role_id", role_id)
                .field("name", name)
                .field("permissions", permissions)
                .field("color", color)
                .field("position", position)
                .field("hoist", hoist)
                .field("mentionable", mentionable)
                .field("self_assignable", self_assignable)
                .field("exclusion_group", exclusion_group)
                .finish(),
            Self::RoleDelete { community, role_id } => f
                .debug_struct("RoleDelete")
                .field("community", community)
                .field("role_id", role_id)
                .finish(),
            Self::RoleAssign {
                community,
                member_pseudonym,
                role_id,
            } => f
                .debug_struct("RoleAssign")
                .field("community", community)
                .field("member_pseudonym", member_pseudonym)
                .field("role_id", role_id)
                .finish(),
            Self::RoleUnassign {
                community,
                member_pseudonym,
                role_id,
            } => f
                .debug_struct("RoleUnassign")
                .field("community", community)
                .field("member_pseudonym", member_pseudonym)
                .field("role_id", role_id)
                .finish(),
            Self::RoleSelfAssign { community, role_id } => f
                .debug_struct("RoleSelfAssign")
                .field("community", community)
                .field("role_id", role_id)
                .finish(),
            Self::RoleSelfUnassign { community, role_id } => f
                .debug_struct("RoleSelfUnassign")
                .field("community", community)
                .field("role_id", role_id)
                .finish(),
        }
    }
}
