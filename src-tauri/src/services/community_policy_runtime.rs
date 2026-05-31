//! Phase 23.C — community-policy bodies lifted from
//! `commands/community/policy.rs`. Tauri commands stay thin
//! delegations; this file hosts the governance-policy DTO
//! aggregation + the write_entry orchestration for policy mutations.
//! Pure read/write — no protocol logic per Invariant 7 (write_entry
//! itself owns sig + dispatch).

use crate::commands::community::helpers::require_permission;
use crate::commands::community::policy::CommunityPolicyDto;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_governance::state::CommunityPolicyState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_types::governance::GovernanceEntry;

pub fn get_community_policy_inner(
    state: &SharedState,
    community_id: &str,
) -> Result<CommunityPolicyDto, String> {
    require_permission(state, community_id, Permissions::VIEW_CHANNEL)?;
    let policy = state_helpers::governance_state(state, community_id)
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

pub async fn set_community_policy_inner(
    state: &SharedState,
    community_id: &str,
    policy_text: Option<String>,
    max_joins_per_interval: u32,
    join_interval_seconds: u32,
) -> Result<(), String> {
    require_permission(state, community_id, Permissions::MANAGE_COMMUNITY)?;
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
    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        GovernanceEntry::CommunityPolicy {
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
            lamport,
        },
    )
    .await
}
