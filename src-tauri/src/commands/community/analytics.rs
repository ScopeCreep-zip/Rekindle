//! Tauri command for local community analytics (architecture §24.1).
//!
//! Permission gate is enforced here against the merged governance state
//! so a non-admin client can't bypass UI gating to fetch insights.

use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_types::analytics::CommunityAnalytics;
use tauri::State;

use crate::commands::community::require_permission;
use crate::db::DbPool;
use crate::services::community::analytics;
use crate::state::SharedState;

#[tauri::command]
pub async fn get_community_analytics(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<CommunityAnalytics, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_INSIGHTS)?;
    analytics::compute_community_analytics(state.inner(), pool.inner(), &community_id).await
}
