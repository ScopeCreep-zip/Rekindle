//! The voice stage's hand raises, over the Host's channel adapter
//! (`HostChannel`) until the voice signaling moves (plan E4.1).

use std::sync::Arc;

use crate::state::AppState;

pub async fn list_hand_raises(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<String>, String> {
    crate::state_helpers::host_channel(state)
        .map_err(|e| e.to_string())?
        .hand_raises(community_id, channel_id)
        .await
        .map_err(|e| e.to_string())
}
