//! The role commands on both modes (plan E3.1 F1): list, create, edit,
//! delete, assign, unassign, and a member's own take and drop of a
//! self-assignable role.
//!
//! Each write is checked here the way every reader checks it after the
//! merge (reader-validates, `rekindle_governance::validate`): our own
//! effective permissions from the merged state (`require_permission_impl`),
//! and for the self paths the role's `self_assignable` flag. The entry is
//! then written and merged by `rekindle_governance_runtime::roles`, and the
//! projection lays it onto the community store (`roles.rs`).

use std::fmt;

use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::roles::{self, ExclusionGroupEdit, RoleSnapshotPatch};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::display::RoleDisplay;
use rekindle_types::id::RoleId;
use rekindle_types::permissions::MANAGE_ROLES;

use super::HostGovernance;

/// The longest exclusion-group slug (architecture §19.4).
const MAX_EXCLUSION_GROUP_CHARS: usize = 32;

/// A new role's attributes, as `RoleCreate` carries them.
#[derive(Debug, Clone)]
pub struct NewRole {
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub hoist: bool,
    pub mentionable: bool,
    pub self_assignable: bool,
    pub exclusion_group: Option<String>,
}

/// Why a role command did not run.
#[derive(Debug)]
pub enum RoleRefusal {
    /// Our merged permissions lack `MANAGE_ROLES`, or the role ranks too high.
    NotPermitted,
    /// The role is not one members may take or drop themselves.
    NotSelfAssignable(u32),
    /// No live role has this id.
    UnknownRole(u32),
    /// We hold no pseudonym in this community.
    NotAMember,
    /// The exclusion-group slug is malformed.
    BadExclusionGroup(&'static str),
    /// The governance write failed.
    Failed(GovernanceRuntimeError),
}

impl RoleRefusal {
    /// The IPC status the refusal answers with.
    #[must_use]
    pub fn code(&self) -> u16 {
        match self {
            Self::NotPermitted | Self::NotSelfAssignable(_) => 403,
            Self::UnknownRole(_) | Self::NotAMember => 404,
            Self::BadExclusionGroup(_) => 400,
            Self::Failed(_) => 500,
        }
    }
}

impl fmt::Display for RoleRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPermitted => f.write_str("missing permission: manage roles"),
            Self::NotSelfAssignable(id) => write!(f, "role {id} is not self-assignable"),
            Self::UnknownRole(id) => write!(f, "role not found: {id}"),
            Self::NotAMember => f.write_str("no pseudonym for this community"),
            Self::BadExclusionGroup(why) => write!(f, "exclusion_group {why}"),
            Self::Failed(e) => write!(f, "role write: {e}"),
        }
    }
}

impl From<GovernanceRuntimeError> for RoleRefusal {
    fn from(e: GovernanceRuntimeError) -> Self {
        match e {
            GovernanceRuntimeError::PermissionDenied => Self::NotPermitted,
            other => Self::Failed(other),
        }
    }
}

/// Every live role of `governance`, by id.
#[must_use]
pub fn role_displays(governance: &GovernanceState) -> Vec<RoleDisplay> {
    let mut out: Vec<RoleDisplay> = governance
        .roles
        .iter()
        .map(|(id, role)| RoleDisplay {
            id: id.to_legacy_u32(),
            name: role.name.clone(),
            color: role.color,
            permissions: role.permissions,
            position: i32::try_from(role.position).unwrap_or(i32::MAX),
            hoist: role.hoist,
            mentionable: role.mentionable,
            self_assignable: role.self_assignable,
            exclusion_group: role.exclusion_group.clone(),
        })
        .collect();
    out.sort_by_key(|r| r.id);
    out
}

/// A slug lowercased, or `None` when blank.
///
/// # Errors
/// Longer than 32 characters, or other than letters, digits, `_` and `-`.
pub fn normalize_exclusion_group(raw: Option<&str>) -> Result<Option<String>, RoleRefusal> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > MAX_EXCLUSION_GROUP_CHARS {
        return Err(RoleRefusal::BadExclusionGroup(
            "must be at most 32 characters",
        ));
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(RoleRefusal::BadExclusionGroup(
            "may only contain letters, numbers, '_' or '-'",
        ));
    }
    Ok(Some(trimmed.to_ascii_lowercase()))
}

/// The edit with its slug normalized; a blank `Set` is a `Clear`.
fn normalize_edit(edit: ExclusionGroupEdit) -> Result<ExclusionGroupEdit, RoleRefusal> {
    Ok(match edit {
        ExclusionGroupEdit::Set(value) => match normalize_exclusion_group(Some(&value))? {
            Some(slug) => ExclusionGroupEdit::Set(slug),
            None => ExclusionGroupEdit::Clear,
        },
        other => other,
    })
}

/// A member may take or drop `role_id` themselves only when it is live
/// and marked `self_assignable` — the rule readers apply to the entry
/// (`validate::can_self_assign_role`).
///
/// # Errors
/// The role is unknown or not self-assignable.
pub fn self_assignable(governance: &GovernanceState, role_id: u32) -> Result<(), RoleRefusal> {
    match governance.roles.get(&RoleId::from_legacy_u32(role_id)) {
        None => Err(RoleRefusal::UnknownRole(role_id)),
        Some(role) if !role.self_assignable => Err(RoleRefusal::NotSelfAssignable(role_id)),
        Some(_) => Ok(()),
    }
}

impl HostGovernance {
    /// The community's live roles.
    #[must_use]
    pub fn role_list(&self, community_id: &str) -> Vec<RoleDisplay> {
        self.governance_state_impl(community_id)
            .map(|g| role_displays(&g))
            .unwrap_or_default()
    }

    /// Create a role; its id.
    ///
    /// # Errors
    /// Not permitted, a bad slug, or the write failed.
    pub async fn role_create(&self, community_id: &str, role: NewRole) -> Result<u32, RoleRefusal> {
        self.require_permission_impl(community_id, MANAGE_ROLES)?;
        let exclusion_group = normalize_exclusion_group(role.exclusion_group.as_deref())?;
        Ok(roles::create_role(
            self,
            community_id,
            role.name.trim().to_owned(),
            role.color,
            role.permissions,
            role.hoist,
            role.mentionable,
            role.self_assignable,
            exclusion_group,
        )
        .await?)
    }

    /// Edit a role.
    ///
    /// # Errors
    /// Not permitted, a bad slug, or the write failed.
    pub async fn role_edit(
        &self,
        community_id: &str,
        role_id: u32,
        mut patch: RoleSnapshotPatch,
    ) -> Result<(), RoleRefusal> {
        self.require_permission_impl(community_id, MANAGE_ROLES)?;
        patch.name = patch.name.map(|n| n.trim().to_owned());
        patch.exclusion_group = normalize_edit(patch.exclusion_group)?;
        Ok(roles::edit_role(self, community_id, role_id, patch).await?)
    }

    /// Archive a role.
    ///
    /// # Errors
    /// Not permitted, or the write failed.
    pub async fn role_delete(&self, community_id: &str, role_id: u32) -> Result<(), RoleRefusal> {
        self.require_permission_impl(community_id, MANAGE_ROLES)?;
        Ok(roles::delete_role(self, community_id, role_id).await?)
    }

    /// Grant `member` a role (additive).
    ///
    /// # Errors
    /// Not permitted, or the write failed.
    pub async fn role_assign(
        &self,
        community_id: &str,
        member: &str,
        role_id: u32,
    ) -> Result<(), RoleRefusal> {
        self.require_permission_impl(community_id, MANAGE_ROLES)?;
        Ok(roles::assign_role(self, community_id, member, role_id).await?)
    }

    /// Take a role from `member`.
    ///
    /// # Errors
    /// Not permitted, or the write failed.
    pub async fn role_unassign(
        &self,
        community_id: &str,
        member: &str,
        role_id: u32,
    ) -> Result<(), RoleRefusal> {
        self.require_permission_impl(community_id, MANAGE_ROLES)?;
        Ok(roles::unassign_role(self, community_id, member, role_id).await?)
    }

    /// Take (`take`) or drop a self-assignable role ourselves.
    ///
    /// # Errors
    /// The role is unknown or not self-assignable, we hold no pseudonym,
    /// or the write failed.
    pub async fn role_self(
        &self,
        community_id: &str,
        role_id: u32,
        take: bool,
    ) -> Result<(), RoleRefusal> {
        let governance = self
            .governance_state_impl(community_id)
            .ok_or(RoleRefusal::UnknownRole(role_id))?;
        self_assignable(&governance, role_id)?;
        let me = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .ok_or(RoleRefusal::NotAMember)?;
        if take {
            roles::assign_role(self, community_id, &me, role_id).await?;
        } else {
            roles::unassign_role(self, community_id, &me, role_id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "role_ops_tests.rs"]
mod tests;
