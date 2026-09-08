//! Phase 23.C — channel-creation runtime orchestration lifted from
//! `commands/community/channels.rs`. Same pattern as the sibling
//! `community_*_runtime.rs` modules: legitimate Tauri-runtime glue
//! (AppState mutation + SQLite persist), no protocol decisions in the
//! body. Record creation and the `ChannelCreated` write moved into
//! `rekindle_governance_runtime::channels`, which the daemon calls too.

use std::sync::Arc;

use rekindle_types::permissions;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::{AppState, ChannelType, SharedState};
use crate::state_helpers;

pub async fn create_channel_inner(
    state: Arc<AppState>,
    pool: DbPool,
    community_id: String,
    name: String,
    channel_type: String,
    category_id: Option<String>,
    parent_voice_channel_id: Option<String>,
) -> Result<String, String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};

    require_permission(&state, &community_id, permissions::MANAGE_CHANNELS)?;
    let owner_key = state_helpers::current_owner_key(&state)?;
    let next_position = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community not found")?;
        community.governance_state.as_ref().map_or_else(
            || u32::try_from(community.channels.len()).unwrap_or(u32::MAX),
            |gov| {
                gov.channels
                    .values()
                    .map(|channel| channel.position)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1)
            },
        )
    };

    let parsed_category_id = category_id
        .as_deref()
        .map(|id| rekindle_types::id::CategoryId(hex_to_id_16(id)));
    let parsed_parent_voice = parent_voice_channel_id
        .as_deref()
        .map(|id| rekindle_types::id::ChannelId(hex_to_id_16(id)));

    // Record creation + `ChannelCreated` in one call, shared with the
    // daemon. Both shells were assembling this by hand, and only one of
    // them got it right: the daemon appended to the v1.0 manifest
    // channels subkey with no record key at all.
    let app_handle = state_helpers::app_handle(&state).ok_or("app handle not initialized")?;
    let adapter = crate::services::governance_adapter::GovernanceAdapter::new(
        Arc::clone(&state),
        app_handle,
        pool.clone(),
    );
    let created = rekindle_governance_runtime::channels::create_channel(
        &adapter,
        &community_id,
        rekindle_governance_runtime::channels::NewChannel {
            name: name.clone(),
            channel_type: channel_type.clone(),
            category_id: parsed_category_id,
            position: next_position,
            parent_voice_channel_id: parsed_parent_voice,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    let channel_id = created.channel_id_hex.clone();
    let record_key = created.record_key.clone();
    state_helpers::track_open_records(&state, std::slice::from_ref(&record_key));

    let channel_type: ChannelType = channel_type.parse().unwrap_or(ChannelType::Text);
    let channel = crate::state::ChannelInfo {
        id: channel_id.clone(),
        name: name.clone(),
        channel_type,
        unread_count: 0,
        category_id,
        topic: String::new(),
        forum_tags: None,
        stage_speakers: Vec::new(),
        stage_moderator: None,
        slowmode_seconds: None,
        nsfw: false,
        message_record_key: Some(record_key.clone()),
        mek_generation: 0,
        notification_level: "all".to_string(),
        notification_sound_ref: None,
        parent_voice_channel_id: parent_voice_channel_id.clone(),
    };

    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            community
                .channel_log_keys
                .insert(channel_id.clone(), record_key.clone());
            community
                .open_community_records
                .channel_keys
                .push(record_key.clone());
        }
    }

    let comm_id = community_id.clone();
    db_call(&pool, move |conn| {
        crate::channel_repo::insert_channel(conn, &owner_key, &channel, &comm_id)?;
        Ok(())
    })
    .await?;

    Ok(channel_id)
}

pub async fn create_category_inner(
    state: &SharedState,
    community_id: String,
    name: String,
) -> Result<String, String> {
    use crate::commands::community::helpers::{random_16_bytes, require_permission};
    use rekindle_types::permissions;

    require_permission(state, &community_id, permissions::MANAGE_CHANNELS)?;
    let next_position = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community not found")?;
        community.governance_state.as_ref().map_or_else(
            || u32::try_from(community.categories.len()).unwrap_or(u32::MAX),
            |gov| {
                gov.categories
                    .values()
                    .map(|category| category.position)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1)
            },
        )
    };
    let category_id_bytes = random_16_bytes();
    let category_id = hex::encode(category_id_bytes);
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::CategoryCreated {
            category_id: rekindle_types::id::CategoryId(category_id_bytes),
            name: name.clone(),
            position: next_position,
            lamport,
        },
    )
    .await?;
    Ok(category_id)
}

pub async fn delete_category_inner(
    state: &SharedState,
    community_id: String,
    category_id: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use rekindle_types::permissions;

    require_permission(state, &community_id, permissions::MANAGE_CHANNELS)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::CategoryArchived {
            category_id: rekindle_types::id::CategoryId(hex_to_id_16(&category_id)),
            lamport,
        },
    )
    .await?;

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        community
            .categories
            .retain(|category| category.id != category_id);
        for channel in &mut community.channels {
            if channel.category_id.as_deref() == Some(&category_id) {
                channel.category_id = None;
            }
        }
    }
    Ok(())
}

pub async fn rename_category_inner(
    state: &SharedState,
    community_id: String,
    category_id: String,
    new_name: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use rekindle_types::permissions;

    require_permission(state, &community_id, permissions::MANAGE_CHANNELS)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::CategoryUpdated {
            category_id: rekindle_types::id::CategoryId(hex_to_id_16(&category_id)),
            name: Some(new_name.clone()),
            position: None,
            lamport,
        },
    )
    .await?;

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(category) = community
            .categories
            .iter_mut()
            .find(|category| category.id == category_id)
        {
            category.name = new_name;
        }
    }
    Ok(())
}

pub async fn move_channel_inner(
    state: &SharedState,
    community_id: String,
    channel_id: String,
    category_id: Option<String>,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use rekindle_types::permissions;

    require_permission(state, &community_id, permissions::MANAGE_CHANNELS)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    let parsed_category_id = category_id
        .as_deref()
        .map(|category| rekindle_types::id::CategoryId(hex_to_id_16(category)));
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: None,
            topic: None,
            forum_tags: None,
            position: None,
            slowmode_seconds: None,
            nsfw: None,
            category_id: Some(parsed_category_id),
            lamport,
        },
    )
    .await?;

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(channel) = community
            .channels
            .iter_mut()
            .find(|channel| channel.id == channel_id)
        {
            channel.category_id = category_id;
        }
    }
    Ok(())
}

pub async fn reorder_categories_inner(
    state: &SharedState,
    community_id: String,
    category_ids: Vec<String>,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use rekindle_types::permissions;

    require_permission(state, &community_id, permissions::MANAGE_CHANNELS)?;
    for (index, category_id) in category_ids.iter().enumerate() {
        let lamport = state_helpers::increment_lamport(state, &community_id);
        crate::services::community::write_entry(
            state,
            &community_id,
            rekindle_types::governance::GovernanceEntry::CategoryUpdated {
                category_id: rekindle_types::id::CategoryId(hex_to_id_16(category_id)),
                name: None,
                position: Some(u32::try_from(index).unwrap_or(u32::MAX)),
                lamport,
            },
        )
        .await?;
    }

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        for (index, category_id) in category_ids.iter().enumerate() {
            if let Some(category) = community
                .categories
                .iter_mut()
                .find(|category| category.id == *category_id)
            {
                category.sort_order = i32::try_from(index).unwrap_or(i32::MAX);
            }
        }
        community
            .categories
            .sort_by_key(|category| category.sort_order);
    }
    Ok(())
}

pub async fn set_channel_topic_inner(
    state: &SharedState,
    community_id: String,
    channel_id: String,
    topic: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use rekindle_types::permissions;

    require_permission(state, &community_id, permissions::MANAGE_CHANNELS)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: None,
            topic: Some(topic.clone()),
            forum_tags: None,
            position: None,
            slowmode_seconds: None,
            nsfw: None,
            category_id: None,
            lamport,
        },
    )
    .await?;

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
            ch.topic = topic;
        }
    }
    Ok(())
}

pub async fn set_channel_forum_tags_inner(
    state: &SharedState,
    community_id: String,
    channel_id: String,
    forum_tags: Vec<String>,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use rekindle_types::permissions;

    require_permission(state, &community_id, permissions::MANAGE_CHANNELS)?;
    let tags: Vec<String> = forum_tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .take(32)
        .collect();
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: None,
            topic: None,
            forum_tags: Some(tags.clone()),
            position: None,
            slowmode_seconds: None,
            nsfw: None,
            category_id: None,
            lamport,
        },
    )
    .await?;

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
            ch.forum_tags = if tags.is_empty() { None } else { Some(tags) };
        }
    }
    Ok(())
}

pub async fn reorder_channels_inner(
    state: &SharedState,
    community_id: String,
    channel_ids: Vec<String>,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use rekindle_types::permissions;

    require_permission(state, &community_id, permissions::MANAGE_CHANNELS)?;
    for (i, ch_id) in channel_ids.iter().enumerate() {
        let lamport = state_helpers::increment_lamport(state, &community_id);
        crate::services::community::write_entry(
            state,
            &community_id,
            rekindle_types::governance::GovernanceEntry::ChannelUpdated {
                channel_id: rekindle_types::id::ChannelId(hex_to_id_16(ch_id)),
                name: None,
                topic: None,
                forum_tags: None,
                position: Some(u32::try_from(i).unwrap_or(u32::MAX)),
                slowmode_seconds: None,
                nsfw: None,
                category_id: None,
                lamport,
            },
        )
        .await?;
    }

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        community.channels.sort_by_key(|ch| {
            channel_ids
                .iter()
                .position(|id| id == &ch.id)
                .unwrap_or(usize::MAX)
        });
    }
    Ok(())
}
