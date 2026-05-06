//! Tauri command for Plate Gate (architecture §15) admin expansion.
//!
//! Thin wrapper — all logic in `services/community/segments.rs`.

use tauri::State;

use crate::state::SharedState;

/// Admin-only: create a new SMPL segment when the current highest segment
/// is full. Returns the new `segment_index`. Permission `MANAGE_COMMUNITY`
/// is enforced by both the service and `rekindle-governance::validate`
/// (reader-validates per architecture §15.2).
#[tauri::command]
pub async fn expand_community_segment(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<u32, String> {
    crate::services::community::segments::expand_community_segment(state.inner(), &community_id).await
}
