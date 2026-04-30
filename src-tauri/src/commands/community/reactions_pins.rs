use tauri::State;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::require_permission;
use super::types::PinnedMessageInfoDto;

/// Add a reaction to a community channel message.
#[tauri::command]
pub async fn add_reaction(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::SEND_MESSAGES)?;
    crate::services::community::persist_reaction(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &emoji,
        true,
    )
    .await
}

/// Remove a reaction from a community channel message.
#[tauri::command]
pub async fn remove_reaction(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::SEND_MESSAGES)?;
    crate::services::community::persist_reaction(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &emoji,
        false,
    )
    .await
}

/// Pin a message in a community channel.
#[tauri::command]
pub async fn pin_message(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    message_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_MESSAGES)?;
    let _ = pool;
    let pinned_by = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::MessagePinned {
            channel_id,
            message_id,
            pinned_by,
        }),
    )
}

/// Unpin a message from a community channel.
#[tauri::command]
pub async fn unpin_message(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    message_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_MESSAGES)?;
    let _ = pool;
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::MessageUnpinned {
            channel_id,
            message_id,
        }),
    )
}

/// Get pinned messages for a community channel.
#[tauri::command]
pub async fn get_channel_pins(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<PinnedMessageInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
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
