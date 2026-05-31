//! Phase 23.C — role-command Tauri handler wrappers lifted from
//! `commands/community/roles.rs`. Each wrapper handles the
//! input-parsing + permission-check + delegate dance that the Tauri
//! command would otherwise carry inline; the underlying CRDT/state
//! mutation lives in `community_role_runtime.rs`. No protocol logic
//! lives here — just runtime glue per Invariant 7.

use crate::commands::community::helpers::require_permission;
use crate::db::DbPool;
use crate::services::community_role_runtime::{create_role_inner, edit_role_inner};
use crate::state::SharedState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

/// Architecture §19.4 — explicit edit verb so callers can either set
/// a new exclusion-group slug or clear it. Omitting the field
/// entirely on `edit_role` is "no change".
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum ExclusionGroupEdit {
    Set(String),
    Clear,
}

/// Architecture §19.4 — exclusion-group slugs are case-insensitive
/// short tags. Normalise to lowercase and reject overlong / control
/// chars; treat `Some("")` as `None`.
pub fn normalize_exclusion_group(raw: Option<String>) -> Result<Option<String>, String> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if trimmed.chars().count() > 32 {
                return Err("exclusion_group must be ≤32 characters".into());
            }
            if !trimmed
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            {
                return Err("exclusion_group may only contain letters, numbers, '_' or '-'".into());
            }
            Ok(Some(trimmed.to_ascii_lowercase()))
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches RoleDefinition shape"
)]
pub async fn create_role_handler_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    name: String,
    color: u32,
    permissions: String,
    hoist: bool,
    mentionable: bool,
    self_assignable: bool,
    exclusion_group: Option<String>,
) -> Result<u32, String> {
    require_permission(state, &community_id, Permissions::MANAGE_ROLES)?;
    let permissions_u64: u64 = permissions
        .parse()
        .map_err(|e| format!("invalid permissions: {e}"))?;
    let exclusion_group = normalize_exclusion_group(exclusion_group)?;
    create_role_inner(
        state,
        pool,
        community_id,
        name,
        color,
        permissions_u64,
        hoist,
        mentionable,
        self_assignable,
        exclusion_group,
    )
    .await
}

#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches edit_role partial-update payload"
)]
pub async fn edit_role_handler_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    role_id: u32,
    name: Option<String>,
    color: Option<u32>,
    permissions: Option<String>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
    self_assignable: Option<bool>,
    exclusion_group: Option<ExclusionGroupEdit>,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_ROLES)?;
    let permissions_u64: Option<u64> = permissions
        .map(|s| s.parse::<u64>())
        .transpose()
        .map_err(|e| format!("invalid permissions: {e}"))?;
    let normalised_exclusion: rekindle_governance_runtime::roles::ExclusionGroupEdit =
        match exclusion_group {
            None => rekindle_governance_runtime::roles::ExclusionGroupEdit::Unchanged,
            Some(ExclusionGroupEdit::Clear) => {
                rekindle_governance_runtime::roles::ExclusionGroupEdit::Clear
            }
            Some(ExclusionGroupEdit::Set(value)) => match normalize_exclusion_group(Some(value))? {
                None => rekindle_governance_runtime::roles::ExclusionGroupEdit::Clear,
                Some(s) => rekindle_governance_runtime::roles::ExclusionGroupEdit::Set(s),
            },
        };
    edit_role_inner(
        state,
        pool,
        community_id,
        role_id,
        name,
        color,
        permissions_u64,
        position,
        hoist,
        mentionable,
        self_assignable,
        normalised_exclusion,
    )
    .await
}
