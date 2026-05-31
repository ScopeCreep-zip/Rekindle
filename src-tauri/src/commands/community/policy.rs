//! Architecture §10.7 + §20.6 — `CommunityPolicy` Tauri commands.
//!
//! Read accessors fall back to architecture defaults when no
//! `CommunityPolicy` entry has been merged yet. Writes require
//! `MANAGE_COMMUNITY` (enforced by `rekindle_governance::validate`
//! reader-side; we also pre-flight here so the user gets a fast error
//! instead of a silent rejection).

use tauri::State;

use crate::state::SharedState;

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
    crate::services::community_policy_runtime::get_community_policy_inner(
        state.inner(),
        &community_id,
    )
}

#[tauri::command]
pub async fn set_community_policy(
    state: State<'_, SharedState>,
    community_id: String,
    policy_text: Option<String>,
    max_joins_per_interval: u32,
    join_interval_seconds: u32,
) -> Result<(), String> {
    crate::services::community_policy_runtime::set_community_policy_inner(
        state.inner(),
        &community_id,
        policy_text,
        max_joins_per_interval,
        join_interval_seconds,
    )
    .await
}
