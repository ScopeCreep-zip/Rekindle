use tauri::State;
use veilid_core::CRYPTO_KIND_VLD0;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::{ChannelType, SharedState};
use crate::state_helpers;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_records::schema;
use rekindle_secrets::derive;

use super::helpers::{hex_to_id_16, random_16_bytes, require_permission};

#[tauri::command]
pub async fn create_channel(
    community_id: String,
    name: String,
    channel_type: String,
    category_id: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let rc = state_helpers::routing_context(state.inner()).ok_or("not attached")?;
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

    let channel_id_bytes = random_16_bytes();
    let channel_id = hex::encode(channel_id_bytes);
    let parsed_category_id = category_id
        .as_deref()
        .map(|id| rekindle_types::id::CategoryId(hex_to_id_16(id)));

    let slot_seed_hex = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|community| community.slot_seed.clone())
            .ok_or("no slot seed available for community")?
    };
    let slot_seed_bytes: [u8; 32] = hex::decode(&slot_seed_hex)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes")?;
    let mut member_pubkeys = Vec::with_capacity(schema::MAX_MEMBERS_PER_SEGMENT);
    for index in 0..schema::MAX_MEMBERS_PER_SEGMENT {
        let keypair = derive::derive_slot_keypair(
            &slot_seed_bytes,
            u32::try_from(index).map_err(|_| "slot index overflow")?,
        )
        .map_err(|e| format!("slot keypair derivation failed at index {index}: {e}"))?;
        member_pubkeys.push(keypair.verifying_key().to_bytes());
    }
    let channel_schema = schema::community_smpl_schema(&member_pubkeys)
        .map_err(|e| format!("channel schema creation failed: {e}"))?;
    let channel_desc = rc
        .create_dht_record(CRYPTO_KIND_VLD0, channel_schema, None)
        .await
        .map_err(|e| format!("channel record creation failed: {e}"))?;
    let record_key = channel_desc.key().to_string();
    state_helpers::track_open_records(state.inner(), std::slice::from_ref(&record_key));

    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelCreated {
            channel_id: rekindle_types::id::ChannelId(channel_id_bytes),
            name: name.clone(),
            channel_type: channel_type.clone(),
            record_key: record_key.clone(),
            category_id: parsed_category_id,
            position: next_position,
            lamport,
        },
    )
    .await?;

    let channel_type: ChannelType = channel_type.parse().unwrap_or(ChannelType::Text);
    let channel = crate::state::ChannelInfo {
        id: channel_id.clone(),
        name: name.clone(),
        channel_type,
        unread_count: 0,
        category_id,
        topic: String::new(),
        slowmode_seconds: None,
        nsfw: false,
        message_record_key: Some(record_key.clone()),
        mek_generation: 0,
        notification_level: "all".to_string(),
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
    db_call(pool.inner(), move |conn| {
        crate::channel_repo::insert_channel(conn, &owner_key, &channel, &comm_id)?;
        Ok(())
    })
    .await?;

    Ok(channel_id)
}

#[tauri::command]
pub async fn create_category(
    community_id: String,
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
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
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
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

#[tauri::command]
pub async fn delete_category(
    community_id: String,
    category_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
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

#[tauri::command]
pub async fn rename_category(
    community_id: String,
    category_id: String,
    new_name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
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

#[tauri::command]
pub async fn move_channel(
    community_id: String,
    channel_id: String,
    category_id: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    let parsed_category_id = category_id
        .as_deref()
        .map(|category| rekindle_types::id::CategoryId(hex_to_id_16(category)));
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: None,
            topic: None,
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

#[tauri::command]
pub async fn reorder_categories(
    community_id: String,
    category_ids: Vec<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    for (index, category_id) in category_ids.iter().enumerate() {
        let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
        crate::services::community::write_entry(
            state.inner(),
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

#[tauri::command]
pub async fn set_channel_topic(
    community_id: String,
    channel_id: String,
    topic: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: None,
            topic: Some(topic.clone()),
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

#[tauri::command]
pub async fn reorder_channels(
    community_id: String,
    channel_ids: Vec<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    for (i, ch_id) in channel_ids.iter().enumerate() {
        let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
        crate::services::community::write_entry(
            state.inner(),
            &community_id,
            rekindle_types::governance::GovernanceEntry::ChannelUpdated {
                channel_id: rekindle_types::id::ChannelId(hex_to_id_16(ch_id)),
                name: None,
                topic: None,
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
