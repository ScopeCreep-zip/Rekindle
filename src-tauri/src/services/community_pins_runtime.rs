//! Phase 23.C — pinned-message handler orchestration lifted from
//! `commands/community/reactions_pins.rs`. Hosts `pin_message`,
//! `unpin_message`, and `get_channel_pins` bodies.

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use crate::commands::community::helpers::require_permission;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedMessageInfoDto {
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: u64,
}

pub fn pin_message_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: String,
    message_id: String,
) -> Result<(), String> {
    require_permission(state, community_id, Permissions::MANAGE_MESSAGES)?;
    let pinned_by = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MessagePinned {
            channel_id,
            message_id,
            pinned_by,
        }),
    )
}

pub fn unpin_message_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: String,
    message_id: String,
) -> Result<(), String> {
    require_permission(state, community_id, Permissions::MANAGE_MESSAGES)?;
    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MessageUnpinned {
            channel_id,
            message_id,
        }),
    )
}

pub async fn get_channel_pins_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    channel_id: String,
) -> Result<Vec<PinnedMessageInfoDto>, String> {
    require_permission(state, &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id, channel_id, pinned_by, pinned_at FROM channel_pins \
             WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![owner_key, community_id, channel_id],
            |row| {
                Ok(PinnedMessageInfoDto {
                    message_id: row.get(0)?,
                    channel_id: row.get(1)?,
                    pinned_by: row.get(2)?,
                    pinned_at: row.get::<_, i64>(3).unwrap_or(0).cast_unsigned(),
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}
