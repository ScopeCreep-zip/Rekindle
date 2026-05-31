//! Phase 23.C — channel-admin runtime orchestration lifted from
//! `commands/community/channel_admin.rs`. Hosts the delete + rename
//! handler bodies — governance entry + AppState mutation + SQLite
//! persistence. Sibling to `community_channel_runtime.rs` (which holds
//! the create + category orchestrators).

use rekindle_protocol::dht::community::permissions_v2::Permissions;

use crate::commands::community::helpers::{hex_to_id_16, require_permission};
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

pub async fn delete_channel_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    channel_id: String,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_CHANNELS)?;
    let owner_key = state_helpers::current_owner_key(state)?;

    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelArchived {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            lamport,
        },
    )
    .await?;

    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            community.channels.retain(|ch| ch.id != channel_id);
        }
    }

    let community_id_clone = community_id.clone();
    let channel_id_clone = channel_id.clone();
    db_call(pool, move |conn| {
        crate::channel_repo::delete_channel(
            conn,
            &owner_key,
            &channel_id_clone,
            &community_id_clone,
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, channel = %channel_id, "channel deleted");
    Ok(())
}

pub async fn rename_channel_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    channel_id: String,
    new_name: String,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_CHANNELS)?;
    let owner_key = state_helpers::current_owner_key(state)?;

    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: Some(new_name.clone()),
            topic: None,
            forum_tags: None,
            position: None,
            slowmode_seconds: None,
            nsfw: None,
            category_id: None,
            lamport,
        },
    )
    .await?;

    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            if let Some(ch) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
                ch.name.clone_from(&new_name);
            }
        }
    }

    let community_id_clone = community_id.clone();
    let channel_id_clone = channel_id.clone();
    let name_clone = new_name.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE channels SET name = ? WHERE owner_key = ? AND id = ? AND community_id = ?",
            rusqlite::params![name_clone, owner_key, channel_id_clone, community_id_clone],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, channel = %channel_id, "channel renamed");
    Ok(())
}
