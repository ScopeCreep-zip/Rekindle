//! SQLite snapshot persistence for merged governance state.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;

use super::identity::current_owner_key;

/// Persist the current merged governance snapshot into SQLite for restart hydration.
pub async fn persist_governance_snapshot_to_sqlite(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    lamport_clock: u64,
) -> Result<(), String> {
    #[derive(Clone)]
    struct ChannelRow {
        id: String,
        name: String,
        channel_type: String,
        sort_order: i64,
        category_id: Option<String>,
        topic: String,
        slowmode_seconds: i64,
        nsfw: i32,
        message_record_key: Option<String>,
        mek_generation: i64,
        log_key: Option<String>,
        my_sequence: i64,
    }

    #[derive(Clone)]
    struct RoleRow {
        role_id: i64,
        name: String,
        color: i64,
        permissions: i64,
        position: i64,
        hoist: i32,
        mentionable: i32,
        self_assignable: i32,
        exclusion_group: Option<String>,
    }

    #[derive(Clone)]
    struct CategoryRow {
        id: String,
        name: String,
        sort_order: i64,
    }

    #[derive(Clone)]
    struct OverwriteRow {
        channel_id: String,
        target_type: String,
        target_id: String,
        allow: i64,
        deny: i64,
    }

    let owner_key = current_owner_key(state)?;
    let (
        community_id_owned,
        community_name,
        community_description,
        icon_hash,
        banner_hash,
        my_role_ids_json,
        mek_generation,
        channels,
        roles,
        categories,
        overwrites,
    ) = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        let gov_state = community
            .governance_state
            .clone()
            .ok_or("governance state not loaded")?;
        let metadata = gov_state.metadata.clone();

        let mut channels: Vec<ChannelRow> = gov_state
            .channels
            .iter()
            .map(|(channel_id, channel)| {
                let channel_id_hex = hex::encode(channel_id.0);
                ChannelRow {
                    id: channel_id_hex.clone(),
                    name: channel.name.clone(),
                    channel_type: channel.channel_type.clone(),
                    sort_order: i64::from(channel.position),
                    category_id: channel
                        .category_id
                        .map(|category_id| hex::encode(category_id.0)),
                    topic: channel.topic.clone().unwrap_or_default(),
                    slowmode_seconds: i64::from(channel.slowmode_seconds.unwrap_or(0)),
                    nsfw: i32::from(channel.nsfw.unwrap_or(false)),
                    message_record_key: (!channel.record_key.is_empty())
                        .then(|| channel.record_key.clone()),
                    mek_generation: community.mek_generation.try_into().unwrap_or(i64::MAX),
                    log_key: (!channel.record_key.is_empty()).then(|| channel.record_key.clone()),
                    my_sequence: community
                        .channel_sequences
                        .get(&channel_id_hex)
                        .copied()
                        .unwrap_or(0)
                        .try_into()
                        .unwrap_or(i64::MAX),
                }
            })
            .collect();
        channels.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut roles: Vec<RoleRow> = gov_state
            .roles
            .iter()
            .map(|(role_id, role)| RoleRow {
                role_id: i64::from(rekindle_types::id::RoleId::to_legacy_u32(*role_id)),
                name: role.name.clone(),
                color: i64::from(role.color),
                permissions: role.permissions.cast_signed(),
                position: i64::from(role.position),
                hoist: i32::from(role.hoist),
                mentionable: i32::from(role.mentionable),
                self_assignable: i32::from(role.self_assignable),
                exclusion_group: role.exclusion_group.clone(),
            })
            .collect();
        roles.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut categories: Vec<CategoryRow> = gov_state
            .categories
            .iter()
            .map(|(category_id, category)| CategoryRow {
                id: hex::encode(category_id.0),
                name: category.name.clone(),
                sort_order: i64::from(category.position),
            })
            .collect();
        categories.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut overwrites: Vec<OverwriteRow> = gov_state
            .overwrites
            .iter()
            .map(|((channel_id, target_id), overwrite)| OverwriteRow {
                channel_id: hex::encode(channel_id.0),
                target_type: overwrite.target_type.clone(),
                target_id: target_id.clone(),
                allow: overwrite.allow.cast_signed(),
                deny: overwrite.deny.cast_signed(),
            })
            .collect();
        overwrites.sort_by(|a, b| {
            a.channel_id
                .cmp(&b.channel_id)
                .then_with(|| a.target_type.cmp(&b.target_type))
                .then_with(|| a.target_id.cmp(&b.target_id))
        });

        let mut my_role_ids = community.my_role_ids.clone();
        my_role_ids.sort_unstable();

        (
            community_id.to_string(),
            community.name.clone(),
            community.description.clone(),
            metadata.as_ref().and_then(|meta| meta.icon_hash.clone()),
            metadata.as_ref().and_then(|meta| meta.banner_hash.clone()),
            serde_json::to_string(&my_role_ids).unwrap_or_else(|_| "[0]".to_string()),
            community.mek_generation.try_into().unwrap_or(i64::MAX),
            channels,
            roles,
            categories,
            overwrites,
        )
    };

    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE communities SET name = ?1, description = ?2, icon_hash = ?3, banner_hash = ?4, \
             my_role_ids = ?5, mek_generation = ?6, lamport_clock = ?7 \
             WHERE owner_key = ?8 AND id = ?9",
            rusqlite::params![
                community_name,
                community_description,
                icon_hash,
                banner_hash,
                my_role_ids_json,
                mek_generation,
                lamport_clock.cast_signed(),
                owner_key,
                community_id_owned,
            ],
        )?;

        conn.execute(
            "DELETE FROM channels WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id_owned],
        )?;
        for channel in &channels {
            conn.execute(
                "INSERT INTO channels \
                 (owner_key, id, community_id, name, channel_type, sort_order, category_id, topic, slowmode_seconds, nsfw, message_record_key, mek_generation, log_key, my_sequence) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                rusqlite::params![
                    owner_key,
                    channel.id,
                    community_id_owned,
                    channel.name,
                    channel.channel_type,
                    channel.sort_order,
                    channel.category_id,
                    channel.topic,
                    channel.slowmode_seconds,
                    channel.nsfw,
                    channel.message_record_key,
                    channel.mek_generation,
                    channel.log_key,
                    channel.my_sequence,
                ],
            )?;
        }

        conn.execute(
            "DELETE FROM community_roles WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id_owned],
        )?;
        for role in &roles {
            conn.execute(
                "INSERT INTO community_roles \
                 (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable, exclusion_group) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    owner_key,
                    community_id_owned,
                    role.role_id,
                    role.name,
                    role.color,
                    role.permissions,
                    role.position,
                    role.hoist,
                    role.mentionable,
                    role.self_assignable,
                    role.exclusion_group,
                ],
            )?;
        }

        conn.execute(
            "DELETE FROM community_categories WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id_owned],
        )?;
        for category in &categories {
            conn.execute(
                "INSERT INTO community_categories (owner_key, community_id, id, name, sort_order) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    owner_key,
                    community_id_owned,
                    category.id,
                    category.name,
                    category.sort_order,
                ],
            )?;
        }

        conn.execute(
            "DELETE FROM channel_overwrites WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id_owned],
        )?;
        for overwrite in &overwrites {
            conn.execute(
                "INSERT INTO channel_overwrites \
                 (owner_key, community_id, channel_id, target_type, target_id, allow, deny) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    owner_key,
                    community_id_owned,
                    overwrite.channel_id,
                    overwrite.target_type,
                    overwrite.target_id,
                    overwrite.allow,
                    overwrite.deny,
                ],
            )?;
        }

        Ok(())
    })
    .await
}
