use tauri::{Emitter as _, State};

use crate::channels::community_channel::CommunityEvent;
use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::keystore::KeystoreHandle;
use crate::services;
use crate::state::SharedState;
use crate::state_helpers;

use super::legacy::clear_registry_presence_slot;
use super::types::{
    CategoryInfoDto, ChannelInfoDto, CommunityDetail, CommunityInfo, CommunityRoleDto,
};

#[tauri::command]
pub async fn get_communities(state: State<'_, SharedState>) -> Result<Vec<CommunityInfo>, String> {
    let communities = state.communities.read();
    Ok(communities
        .values()
        .map(|community| CommunityInfo {
            id: community.id.clone(),
            name: community.name.clone(),
            description: community.description.clone(),
            channel_count: community.channels.len(),
            // Plan §Failure 4 — display role derived on demand from
            // (my_role_ids, roles); no separate stored field.
            my_role: Some(crate::state::display_role_name(
                &community.my_role_ids,
                &community.roles,
            )),
        })
        .collect())
}

#[tauri::command]
pub async fn get_community_details(
    state: State<'_, SharedState>,
) -> Result<Vec<CommunityDetail>, String> {
    let communities = state.communities.read();
    Ok(communities
        .values()
        .map(|community| CommunityDetail {
            id: community.id.clone(),
            name: community.name.clone(),
            description: community.description.clone(),
            icon_hash: community.icon_hash.clone(),
            banner_hash: community.banner_hash.clone(),
            channels: community
                .channels
                .iter()
                .map(|channel| ChannelInfoDto {
                    id: channel.id.clone(),
                    name: channel.name.clone(),
                    channel_type: channel.channel_type.to_string(),
                    unread_count: channel.unread_count,
                    category_id: channel.category_id.clone(),
                    topic: channel.topic.clone(),
                    forum_tags: channel.forum_tags.clone(),
                    stage_speakers: channel.stage_speakers.clone(),
                    stage_moderator: channel.stage_moderator.clone(),
                    slowmode_seconds: channel.slowmode_seconds,
                    notification_level: channel.notification_level.clone(),
                    notification_sound_ref: channel.notification_sound_ref.clone(),
                })
                .collect(),
            categories: community
                .categories
                .iter()
                .map(|category| CategoryInfoDto {
                    id: category.id.clone(),
                    name: category.name.clone(),
                    sort_order: category.sort_order,
                })
                .collect(),
            my_role: Some(crate::state::display_role_name(
                &community.my_role_ids,
                &community.roles,
            )),
            my_role_ids: community.my_role_ids.clone(),
            roles: community.roles.iter().map(CommunityRoleDto::from).collect(),
            my_pseudonym_key: community.my_pseudonym_key.clone(),
            mek_generation: community.mek_generation,
            member_registry_key: community.member_registry_key.clone(),
            governance_key: community.governance_key.clone(),
            onboarding_complete: community.onboarding_complete,
            my_bio: community.my_bio.clone(),
            my_pronouns: community.my_pronouns.clone(),
            my_theme_color: community.my_theme_color,
            my_badges: community.my_badges.clone(),
        })
        .collect())
}

#[tauri::command]
pub async fn create_community(
    _app: tauri::AppHandle,
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<String, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id = services::community::create_community(state.inner(), &name).await?;

    {
        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            let mek_cache = state.mek_cache.lock();
            if let Some(mek) = mek_cache.get(&community_id) {
                crate::keystore::persist_mek(keystore, &community_id, mek);
            }
            drop(mek_cache);

            let communities = state.communities.read();
            if let Some(community) = communities.get(&community_id) {
                if let Some(ref keypair) = community.slot_keypair {
                    crate::keystore::persist_slot_keypair(keystore, &community_id, keypair);
                }
                if let Some(ref seed) = community.slot_seed {
                    crate::keystore::persist_slot_seed(keystore, &community_id, seed);
                }
                if let Some(ref keypair) = community.registry_owner_keypair {
                    crate::keystore::persist_registry_keypair(keystore, &community_id, keypair);
                }
            }
        }
    }

    let community = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .cloned()
            .ok_or("community not found after creation")?
    };

    let creator_key = owner_key.clone();
    let creator_name = state_helpers::identity_display_name(state.inner());
    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|community| community.my_pseudonym_key.clone())
    };

    let now = db::timestamp_now();
    let community_id_clone = community_id.clone();
    let name_clone = name.clone();
    let dht_owner_keypair = community.dht_owner_keypair.clone();
    let member_registry_key_db = community.member_registry_key.clone();
    let governance_key_db = community.governance_key.clone();
    let pseudonym_key = my_pseudonym_key.unwrap_or_else(|| creator_key.clone());
    let roles_to_persist = community.roles.clone();
    let mek_gen = community.mek_generation.cast_signed();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        // Plan §Failure 4 — owner_role_ids carries @everyone (0) AND
        // the Owner role's numeric id (u32::MAX, derived from
        // OWNER_ROLE_ID's first 4 LE bytes). The Owner role definition
        // is persisted via community_roles below; the role_id matches
        // the genesis governance entry.
        let owner_role_ids =
            serde_json::to_string(&[0u32, u32::MAX]).unwrap_or_default();
        conn.execute(
            "INSERT INTO communities (owner_key, id, name, my_role_ids, joined_at, dht_owner_keypair, my_pseudonym_key, mek_generation, member_registry_key, my_subkey_index, my_segment_index, governance_key) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 0, ?)",
            rusqlite::params![ok, community_id_clone, name_clone, owner_role_ids, now, dht_owner_keypair, pseudonym_key, mek_gen, member_registry_key_db, governance_key_db],
        )?;

        for role in &roles_to_persist {
            conn.execute(
                "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable, exclusion_group) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    ok, community_id_clone, role.id, role.name, role.color,
                    role.permissions.cast_signed(), role.position, i32::from(role.hoist), i32::from(role.mentionable),
                    i32::from(role.self_assignable), role.exclusion_group,
                ],
            )?;
        }

        for channel in &community.channels {
            crate::channel_repo::insert_channel(conn, &ok, channel, &community_id_clone)?;
        }

        conn.execute(
            "INSERT OR IGNORE INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, pseudonym_key, creator_name, owner_role_ids, now],
        )?;

        Ok(())
    })
    .await?;

    Ok(community_id)
}

#[tauri::command]
pub async fn join_community(
    community_id: String,
    invite_code: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    services::community::join_community(state.inner(), &community_id, invite_code.as_deref())
        .await?;

    let (
        name,
        my_pseudonym_key,
        mek_generation,
        channels,
        my_role_ids,
        roles_to_persist,
        member_registry_key,
        slot_seed,
        my_subkey_index,
        governance_key_db,
    ) = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community state not found after join")?;
        (
            community.name.clone(),
            community.my_pseudonym_key.clone(),
            community.mek_generation,
            community.channels.clone(),
            community.my_role_ids.clone(),
            community.roles.clone(),
            community.member_registry_key.clone(),
            community.slot_seed.clone(),
            community.my_subkey_index,
            community.governance_key.clone(),
        )
    };

    let pseudonym_key = my_pseudonym_key.unwrap_or_else(|| owner_key.clone());
    let joiner_name = state_helpers::identity_display_name(state.inner());

    {
        let mek_cache = state.mek_cache.lock();
        if let Some(mek) = mek_cache.get(&community_id) {
            let ks = keystore_handle.lock();
            if let Some(ref keystore) = *ks {
                crate::keystore::persist_mek(keystore, &community_id, mek);
            }
        }
    }

    if let Some(ref seed) = slot_seed {
        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            crate::keystore::persist_slot_seed(keystore, &community_id, seed);
        }
    }

    let role_ids_json = serde_json::to_string(&my_role_ids).unwrap_or_else(|_| "[0,1]".to_string());
    let now = db::timestamp_now();
    let community_id_clone = community_id.clone();
    let ok = owner_key;
    let pk = pseudonym_key.clone();
    let mg = mek_generation.cast_signed();
    let rij = role_ids_json;
    let subkey_idx = my_subkey_index.map(i64::from);
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO communities (owner_key, id, name, my_role_ids, joined_at, my_pseudonym_key, mek_generation, member_registry_key, my_subkey_index, my_segment_index, governance_key) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)",
            rusqlite::params![ok, community_id_clone, name, rij, now, pk, mg, member_registry_key, subkey_idx, governance_key_db],
        )?;

        conn.execute(
            "INSERT OR IGNORE INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, pk, joiner_name, rij, now],
        )?;

        for channel in &channels {
            crate::channel_repo::upsert_channel(conn, &ok, channel, &community_id_clone)?;
        }

        for role in &roles_to_persist {
            conn.execute(
                "INSERT OR IGNORE INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable, exclusion_group) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    ok, community_id_clone, role.id, role.name, role.color,
                    role.permissions.cast_signed(), role.position, i32::from(role.hoist), i32::from(role.mentionable),
                    i32::from(role.self_assignable), role.exclusion_group,
                ],
            )?;
        }

        Ok(())
    })
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|community| community.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let _ = crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::MemberLeave {
                pseudonym_key: my_pseudonym_key,
            },
        ),
    );

    let my_pseudonym_key = {
        state
            .communities
            .read()
            .get(&community_id)
            .and_then(|community| community.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    if let Err(error) = clear_registry_presence_slot(
        state.inner(),
        pool.inner(),
        &community_id,
        &my_pseudonym_key,
    )
    .await
    {
        tracing::debug!(
            community = %community_id,
            error = %error,
            "failed to clear local registry slot during leave"
        );
    }

    {
        let record_keys =
            state_helpers::collect_and_clear_community_records(state.inner(), &community_id);
        if !record_keys.is_empty() {
            if let Some(rc) = state_helpers::routing_context(state.inner()) {
                for key_str in &record_keys {
                    if let Ok(record_key) = key_str.parse::<veilid_core::RecordKey>() {
                        let _ = rc.close_dht_record(record_key).await;
                    }
                }
                tracing::debug!(count = record_keys.len(), community = %community_id, "closed community DHT records");
            }
            state_helpers::untrack_records(state.inner(), &record_keys);
        }
    }

    state.mek_cache.lock().remove(&community_id);

    {
        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            crate::keystore::delete_mek(keystore, &community_id);
            crate::keystore::delete_slot_keypair(keystore, &community_id);
            crate::keystore::delete_slot_seed(keystore, &community_id);
            crate::keystore::delete_registry_keypair(keystore, &community_id);
        }
    }

    state.communities.write().remove(&community_id);

    let owner_key = state_helpers::current_owner_key(state.inner())?;
    if !my_pseudonym_key.is_empty() {
        crate::services::community::analytics::log_member_leave(
            pool.inner(),
            &owner_key,
            &community_id,
            &my_pseudonym_key,
        );
    }
    let community_id_clone = community_id.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM communities WHERE owner_key = ? AND id = ?",
            rusqlite::params![owner_key, community_id_clone],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, "left community");
    Ok(())
}

#[tauri::command]
pub async fn update_community_info(
    community_id: String,
    name: Option<String>,
    description: Option<String>,
    icon_hash: Option<String>,
    banner_hash: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // CommunityMeta is LWW per architecture §6.4 — the merged state
    // replaces every field on a winning entry. To avoid nuking
    // icon_hash/banner_hash when only name is changed (and vice
    // versa), preload the current metadata and override only the
    // explicitly supplied fields.
    let (current_name, current_description, current_icon, current_banner) = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.governance_state.as_ref())
            .and_then(|g| g.metadata.as_ref())
            .map_or_else(
                || (None, None, None, None),
                |meta| {
                    (
                        Some(meta.name.clone()),
                        meta.description.clone(),
                        meta.icon_hash.clone(),
                        meta.banner_hash.clone(),
                    )
                },
            )
    };

    let next_name = name.clone().or(current_name);
    let next_description = description.clone().or(current_description);
    let next_icon = icon_hash.clone().or(current_icon);
    let next_banner = banner_hash.clone().or(current_banner);

    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::CommunityMeta {
            name: next_name.clone(),
            description: next_description.clone(),
            icon_hash: next_icon.clone(),
            banner_hash: next_banner.clone(),
            lamport,
        },
    )
    .await?;

    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            if let Some(ref new_name) = name {
                community.name.clone_from(new_name);
            }
            if let Some(ref new_description) = description {
                community.description = Some(new_description.clone());
            }
            if let Some(ref new_icon) = icon_hash {
                community.icon_hash = Some(new_icon.clone());
            }
            if let Some(ref new_banner) = banner_hash {
                community.banner_hash = Some(new_banner.clone());
            }
        }
    }

    let cid = community_id.clone();
    let cid_for_db = cid.clone();
    let name_for_db = name.clone();
    let description_for_db = description.clone();
    let icon_hash_for_db = icon_hash.clone();
    let banner_hash_for_db = banner_hash.clone();
    db_call(pool.inner(), move |conn| {
        if let Some(ref new_name) = name_for_db {
            conn.execute(
                "UPDATE communities SET name = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![new_name, owner_key, cid_for_db],
            )?;
        }
        if let Some(ref new_description) = description_for_db {
            conn.execute(
                "UPDATE communities SET description = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![new_description, owner_key, cid_for_db],
            )?;
        }
        if let Some(ref new_icon) = icon_hash_for_db {
            conn.execute(
                "UPDATE communities SET icon_hash = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![new_icon, owner_key, cid_for_db],
            )?;
        }
        if let Some(ref new_banner) = banner_hash_for_db {
            conn.execute(
                "UPDATE communities SET banner_hash = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![new_banner, owner_key, cid_for_db],
            )?;
        }
        Ok(())
    })
    .await?;

    // Architecture §32 Phase 5 W15 — broadcast the merged community
    // info so every window can refresh without a full
    // `getCommunityDetails`. We send only the fields the caller
    // explicitly changed (matching the Optional semantics of the
    // command); receivers leave unchanged fields alone.
    if let Some(app) = state_helpers::app_handle(state.inner()) {
        let _ = app.emit(
            "community-event",
            CommunityEvent::CommunityUpdated {
                community_id: cid.clone(),
                name: name.clone(),
                description: description.clone(),
                icon_hash: icon_hash.clone(),
                banner_hash: banner_hash.clone(),
            },
        );
    }

    tracing::info!(community = %community_id, "community info updated");
    Ok(())
}
