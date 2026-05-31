//! Phase 23.D.4 — bulk channel-message delete extracted from
//! `community_moderation_runtime.rs` to keep that file under the
//! 500-LoC cap (Invariant 1). Loops `admin_delete_one_message` over a
//! capped slice of message IDs with the same permission gate +
//! per-message error logging the original had.

use crate::commands::community::helpers::require_permission;
use crate::db::DbPool;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::community_moderation_runtime::{admin_delete_one_message, BULK_DELETE_CAP};

pub async fn bulk_delete_channel_messages_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    channel_id: String,
    message_ids: Vec<String>,
    reason: Option<String>,
) -> Result<u32, String> {
    if message_ids.is_empty() {
        return Ok(0);
    }
    if message_ids.len() > BULK_DELETE_CAP {
        return Err(format!(
            "bulk delete capped at {BULK_DELETE_CAP} messages (got {})",
            message_ids.len()
        ));
    }
    require_permission(state, &community_id, Permissions::MANAGE_MESSAGES)?;
    let owner_key = state_helpers::current_owner_key(state)?;

    let mut deleted = 0u32;
    for message_id in &message_ids {
        if let Err(e) = admin_delete_one_message(
            state,
            pool,
            &community_id,
            &channel_id,
            message_id,
            &owner_key,
            reason.clone(),
        )
        .await
        {
            tracing::warn!(
                community = %community_id,
                channel = %channel_id,
                %message_id,
                error = %e,
                "bulk delete: per-message admin delete failed",
            );
        } else {
            deleted += 1;
        }
    }
    Ok(deleted)
}
