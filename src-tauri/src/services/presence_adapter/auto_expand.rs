//! Plate Gate auto-expand trigger (architecture §15.1 / §15.2 / §A5).
//!
//! Admin-side helper: when the highest segment fills up, an online
//! admin's poll tick spawns a background task that creates the next
//! segment. Non-admin joiners hitting a full registry still get the
//! existing "Community is full" error — only admins can self-expand.

use std::sync::Arc;

use crate::state::AppState;

pub(super) fn maybe_auto_expand_segment(state: &Arc<AppState>, community_id: &str) {
    use rekindle_protocol::dht::community::permissions_v2::Permissions;
    if crate::commands::community::require_permission(
        state,
        community_id,
        Permissions::MANAGE_COMMUNITY,
    )
    .is_err()
    {
        return; // not an admin — wait for one to come online
    }
    let state_clone = Arc::clone(state);
    let cid = community_id.to_string();
    tokio::spawn(async move {
        match crate::services::community::segments::highest_segment_full(&state_clone, &cid).await {
            Ok(true) => {
                tracing::info!(
                    community = %cid,
                    "highest segment is full — auto-expanding (admin trigger)",
                );
                if let Err(e) = crate::services::community::segments::expand_community_segment(
                    &state_clone,
                    &cid,
                )
                .await
                {
                    tracing::warn!(
                        community = %cid,
                        error = %e,
                        "auto segment expansion failed — next admin's poll will retry",
                    );
                }
            }
            Ok(false) => {}
            Err(e) => {
                tracing::debug!(
                    community = %cid,
                    error = %e,
                    "highest_segment_full check failed — skipping expansion",
                );
            }
        }
    });
}
