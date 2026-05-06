//! Architecture §10.7 + §20.6 — `CommunityPolicy` Tauri commands.
//!
//! Read accessors fall back to architecture defaults when no
//! `CommunityPolicy` entry has been merged yet. Writes require
//! `MANAGE_COMMUNITY` (enforced by `rekindle_governance::validate`
//! reader-side; we also pre-flight here so the user gets a fast error
//! instead of a silent rejection).

use tauri::State;

use super::helpers::require_permission;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_governance::state::CommunityPolicyState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_types::governance::GovernanceEntry;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityPolicyDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_text: Option<String>,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
}

#[tauri::command]
pub async fn get_community_policy(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<CommunityPolicyDto, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    let policy = state_helpers::governance_state(state.inner(), &community_id)
        .and_then(|gs| gs.community_policy.clone());
    Ok(CommunityPolicyDto {
        policy_text: policy.as_ref().and_then(|p| p.policy_text.clone()),
        max_joins_per_interval: policy.as_ref().map_or(
            CommunityPolicyState::DEFAULT_MAX_JOINS_PER_INTERVAL,
            |p| p.max_joins_per_interval,
        ),
        join_interval_seconds: policy.as_ref().map_or(
            CommunityPolicyState::DEFAULT_JOIN_INTERVAL_SECONDS,
            |p| p.join_interval_seconds,
        ),
    })
}

#[tauri::command]
pub async fn set_community_policy(
    state: State<'_, SharedState>,
    community_id: String,
    policy_text: Option<String>,
    max_joins_per_interval: u32,
    join_interval_seconds: u32,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_COMMUNITY)?;
    if max_joins_per_interval == 0 {
        return Err("max_joins_per_interval must be > 0".into());
    }
    if join_interval_seconds == 0 {
        return Err("join_interval_seconds must be > 0".into());
    }
    if let Some(text) = policy_text.as_ref() {
        if text.chars().count() > 4096 {
            return Err("policy_text exceeds 4096 characters".into());
        }
    }
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        GovernanceEntry::CommunityPolicy {
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
            lamport,
        },
    )
    .await
}
