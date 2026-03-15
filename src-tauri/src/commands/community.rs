use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use crate::channels::ChatEvent;
use crate::commands::chat::Message;
use crate::db::{self, DbPool};
use crate::db_helpers::{db_call, db_fire};
use crate::keystore::KeystoreHandle;
use crate::services;
use crate::state::{ChannelType, SharedState};
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

/// Check that the current user has the given permission for a community.
/// Returns `Ok(())` if the permission is granted, or `Err(...)` with a descriptive message.
/// ADMINISTRATOR always implies all permissions.
pub(crate) fn require_permission(
    state: &SharedState,
    community_id: &str,
    required: Permissions,
) -> Result<(), String> {
    let communities = state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or("community not found")?;
    let my_perms_bits = community
        .my_role_ids
        .iter()
        .filter_map(|rid| community.roles.iter().find(|r| r.id == *rid))
        .fold(0u64, |acc, r| acc | r.permissions);
    let perms = Permissions::from_bits_truncate(my_perms_bits);
    if perms.contains(Permissions::ADMINISTRATOR) || perms.contains(required) {
        Ok(())
    } else {
        Err(format!("missing permission: {required:?}"))
    }
}

/// A community member for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDto {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub display_role: String,
    pub status: String,
    pub timeout_until: Option<u64>,
}

/// A community summary for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channel_count: usize,
    pub my_role: Option<String>,
}

/// Get the list of joined communities.
#[tauri::command]
pub async fn get_communities(state: State<'_, SharedState>) -> Result<Vec<CommunityInfo>, String> {
    let communities = state.communities.read();
    let list = communities
        .values()
        .map(|c| CommunityInfo {
            id: c.id.clone(),
            name: c.name.clone(),
            description: c.description.clone(),
            channel_count: c.channels.len(),
            my_role: c.my_role.clone(),
        })
        .collect();
    Ok(list)
}

/// Channel info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfoDto {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub unread_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub topic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slowmode_seconds: Option<u32>,
}

/// Role DTO for frontend consumption (re-exports the channel's `RoleDto`).
pub use crate::channels::community_channel::RoleDto as CommunityRoleDto;

/// Category info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfoDto {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// Response from creating a community invite.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteCreatedDto {
    /// Raw invite code (16 bytes, hex-encoded = 32 chars).
    pub code: String,
    /// The manifest DHT key for building the invite link.
    pub manifest_key: String,
}

/// Invite info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfoDto {
    pub code_hash: String,
    pub created_by: String,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub expires_at: Option<u64>,
    pub created_at: u64,
    /// Raw invite code (only available for invites this node created, from local SQLite).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Pinned message info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedMessageInfoDto {
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: u64,
}

/// Full community detail with channels for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channels: Vec<ChannelInfoDto>,
    pub categories: Vec<CategoryInfoDto>,
    pub my_role: Option<String>,
    pub my_role_ids: Vec<u32>,
    pub roles: Vec<CommunityRoleDto>,
    pub my_pseudonym_key: Option<String>,
    pub mek_generation: u64,
    pub manifest_key: Option<String>,
    pub member_registry_key: Option<String>,
    pub coordinator_pseudonym: Option<String>,
    pub coordinator_epoch: u64,
}

/// Get all joined communities with full channel details.
#[tauri::command]
pub async fn get_community_details(
    state: State<'_, SharedState>,
) -> Result<Vec<CommunityDetail>, String> {
    let communities = state.communities.read();
    let list = communities
        .values()
        .map(|c| CommunityDetail {
            id: c.id.clone(),
            name: c.name.clone(),
            description: c.description.clone(),
            channels: c
                .channels
                .iter()
                .map(|ch| ChannelInfoDto {
                    id: ch.id.clone(),
                    name: ch.name.clone(),
                    channel_type: ch.channel_type.to_string(),
                    unread_count: ch.unread_count,
                    category_id: ch.category_id.clone(),
                    topic: ch.topic.clone(),
                    slowmode_seconds: ch.slowmode_seconds,
                })
                .collect(),
            categories: c
                .categories
                .iter()
                .map(|cat| CategoryInfoDto {
                    id: cat.id.clone(),
                    name: cat.name.clone(),
                    sort_order: cat.sort_order,
                })
                .collect(),
            my_role: c.my_role.clone(),
            my_role_ids: c.my_role_ids.clone(),
            roles: c.roles.iter().map(CommunityRoleDto::from).collect(),
            my_pseudonym_key: c.my_pseudonym_key.clone(),
            mek_generation: c.mek_generation,
            manifest_key: c.manifest_key.clone(),
            member_registry_key: c.member_registry_key.clone(),
            coordinator_pseudonym: c.coordinator_pseudonym.clone(),
            coordinator_epoch: c.coordinator_epoch,
        })
        .collect();
    Ok(list)
}

/// Create a new community and store it in `AppState` + `SQLite`.
#[tauri::command]
pub async fn create_community(
    _app: tauri::AppHandle,
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<String, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id =
        services::community::create_community(state.inner(), &name).await?;

    // Persist MEK, slot keypair, and slot seed to Stronghold for login restoration
    {
        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            // MEK
            let mek_cache = state.mek_cache.lock();
            if let Some(mek) = mek_cache.get(&community_id) {
                crate::keystore::persist_mek(keystore, &community_id, mek);
            }
            drop(mek_cache);

            // Slot keypair + seed (needed for writing DHT presence)
            let communities = state.communities.read();
            if let Some(c) = communities.get(&community_id) {
                if let Some(ref kp) = c.slot_keypair {
                    crate::keystore::persist_slot_keypair(keystore, &community_id, kp);
                }
                if let Some(ref seed) = c.slot_seed {
                    crate::keystore::persist_slot_seed(keystore, &community_id, seed);
                }
                if let Some(ref mkp) = c.manifest_owner_keypair {
                    crate::keystore::persist_manifest_keypair(keystore, &community_id, mkp);
                }
                if let Some(ref rkp) = c.registry_owner_keypair {
                    crate::keystore::persist_registry_keypair(keystore, &community_id, rkp);
                }
            }
        }
    }

    // Read back the community to get default channel info
    let community = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .cloned()
            .ok_or("community not found after creation")?
    };

    // Read creator identity outside db_call (parking_lot guard is !Send)
    let creator_key = owner_key.clone();
    let creator_name = state_helpers::identity_display_name(state.inner());

    // Get pseudonym key for this community
    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
    };

    let now = db::timestamp_now();
    let community_id_clone = community_id.clone();
    let name_clone = name.clone();
    let dht_record_key = community.dht_record_key.clone();
    let dht_owner_keypair = community.dht_owner_keypair.clone();
    let manifest_key_db = community.manifest_key.clone();
    let member_registry_key_db = community.member_registry_key.clone();
    let pseudonym_key = my_pseudonym_key
        .clone()
        .unwrap_or_else(|| creator_key.clone());
    let roles_to_persist = community.roles.clone();
    let mek_gen = community.mek_generation.cast_signed();
    let coordinator_pseudonym_db = community.coordinator_pseudonym.clone();
    let coordinator_epoch_db = community.coordinator_epoch.cast_signed();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        // Owner gets all default role IDs: @everyone(0), members(1), moderator(2), admin(3), owner(4)
        let owner_role_ids = serde_json::to_string(&[0u32, 1, 2, 3, 4]).unwrap_or_default();
        conn.execute(
            "INSERT INTO communities (owner_key, id, name, my_role, my_role_ids, joined_at, dht_record_key, dht_owner_keypair, my_pseudonym_key, mek_generation, manifest_key, member_registry_key, my_subkey_index, coordinator_pseudonym, coordinator_epoch) \
             VALUES (?, ?, ?, 'owner', ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
            rusqlite::params![ok, community_id_clone, name_clone, owner_role_ids, now, dht_record_key, dht_owner_keypair, pseudonym_key, mek_gen, manifest_key_db, member_registry_key_db, coordinator_pseudonym_db, coordinator_epoch_db],
        )?;

        // Persist roles and channels BEFORE community_members to avoid
        // a race with presence_poll_tick's sync_members_to_state_and_db:
        // that function fires an INSERT into community_members via db_fire
        // which can win the race and cause a plain INSERT here to fail,
        // preventing roles/channels from ever being persisted.
        for r in &roles_to_persist {
            conn.execute(
                "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    ok, community_id_clone, r.id, r.name, r.color,
                    r.permissions.cast_signed(), r.position, i32::from(r.hoist), i32::from(r.mentionable),
                ],
            )?;
        }

        for channel in &community.channels {
            crate::channel_repo::insert_channel(conn, &ok, channel, &community_id_clone)?;
        }

        // Insert creator as first member — OR IGNORE handles the race where
        // sync_members_to_state_and_db already inserted this row.
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

/// Join an existing community via self-service SMPL presence registration.
///
/// Reads manifest from DHT, decrypts invite secrets, derives slot keypair,
/// and starts the gossip mesh. No coordinator needed — zero online members
/// required. The joiner's proof of membership is their valid SMPL presence.
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

    // Read community state populated by join_community (now includes MEK + slot info)
    let (name, dht_record_key, my_pseudonym_key, mek_generation, channels, my_role_ids,
         roles_to_persist, manifest_key, member_registry_key, coordinator_pseudonym_db,
         coordinator_epoch_db, slot_seed, my_subkey_index) = {
        let communities = state.communities.read();
        match communities.get(&community_id) {
            Some(c) => (
                c.name.clone(),
                c.dht_record_key.clone(),
                c.my_pseudonym_key.clone(),
                c.mek_generation,
                c.channels.clone(),
                c.my_role_ids.clone(),
                c.roles.clone(),
                c.manifest_key.clone(),
                c.member_registry_key.clone(),
                c.coordinator_pseudonym.clone(),
                c.coordinator_epoch.cast_signed(),
                c.slot_seed.clone(),
                c.my_subkey_index,
            ),
            None => return Err("community state not found after join".to_string()),
        }
    };
    let pseudonym_key = my_pseudonym_key.unwrap_or_else(|| owner_key.clone());
    let joiner_name = state_helpers::identity_display_name(state.inner());

    // Persist MEK to Stronghold for login restoration
    {
        let mek_cache = state.mek_cache.lock();
        if let Some(mek) = mek_cache.get(&community_id) {
            let ks = keystore_handle.lock();
            if let Some(ref keystore) = *ks {
                crate::keystore::persist_mek(keystore, &community_id, mek);
            }
        }
    }

    // Persist slot_seed to Stronghold for login restoration
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
            "INSERT OR IGNORE INTO communities (owner_key, id, name, my_role, my_role_ids, joined_at, dht_record_key, my_pseudonym_key, mek_generation, manifest_key, member_registry_key, coordinator_pseudonym, coordinator_epoch, my_subkey_index) \
             VALUES (?, ?, ?, 'member', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, name, rij, now, dht_record_key, pk, mg, manifest_key, member_registry_key, coordinator_pseudonym_db, coordinator_epoch_db, subkey_idx],
        )?;

        // Add ourselves to the community_members table
        conn.execute(
            "INSERT OR IGNORE INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, pk, joiner_name, rij, now],
        )?;

        // Persist channels to SQLite
        for channel in &channels {
            crate::channel_repo::upsert_channel(conn, &ok, channel, &community_id_clone)?;
        }

        // Persist roles
        for r in &roles_to_persist {
            conn.execute(
                "INSERT OR IGNORE INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    ok, community_id_clone, r.id, r.name, r.color,
                    r.permissions.cast_signed(), r.position, i32::from(r.hoist), i32::from(r.mentionable),
                ],
            )?;
        }

        Ok(())
    })
    .await?;

    Ok(())
}

/// Create a new channel in a community.
///
/// Persists `CreateChannel` to DHT and broadcasts via gossip. The channel
/// ID is generated locally for optimistic UI; admin peers broadcast the
/// canonical `ChannelCreated` event back to all members.
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

    // Generate channel ID locally for optimistic state update
    let channel_id = format!("ch_{}", hex::encode(&rand_nonce()[..8]));

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::CreateChannel {
            name: name.clone(),
            channel_type: channel_type.clone(),
            category_id: category_id.clone(),
            channel_id: channel_id.clone(),
        },
    )
    .await?;

    // Optimistic local state update
    let ch_type: ChannelType = channel_type.parse().unwrap_or(ChannelType::Text);
    let channel = crate::state::ChannelInfo {
        id: channel_id.clone(),
        name: name.clone(),
        channel_type: ch_type,
        unread_count: 0,
        category_id,
        topic: String::new(),
        slowmode_seconds: None,
        nsfw: false,
        message_record_key: None,
        mek_generation: 0,
    };
    state_helpers::push_community_channel(state.inner(), &community_id, channel.clone());

    // Persist to local SQLite
    let comm_id = community_id.clone();
    db_call(pool.inner(), move |conn| {
        crate::channel_repo::insert_channel(conn, &owner_key, &channel, &comm_id)?;
        Ok(())
    })
    .await?;

    Ok(channel_id)
}

// ---------------------------------------------------------------------------
// Category management
// ---------------------------------------------------------------------------

/// Create a new channel category within a community.
#[tauri::command]
pub async fn create_category(
    community_id: String,
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    // Generate category ID locally for optimistic state update
    let category_id = format!("cat_{}", hex::encode(&rand_nonce()[..8]));

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::CreateCategory { name: name.clone() },
    )
    .await?;

    // Optimistic local state update
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        let sort_order = i32::try_from(community.categories.len()).unwrap_or(i32::MAX);
        community.categories.push(crate::state::CategoryInfo {
            id: category_id.clone(),
            name,
            sort_order,
        });
    }
    Ok(category_id)
}

/// Delete a channel category.
#[tauri::command]
pub async fn delete_category(
    community_id: String,
    category_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::DeleteCategory {
            category_id: category_id.clone(),
        },
    )
    .await?;

    // Optimistic local state update
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        community.categories.retain(|c| c.id != category_id);
        for ch in &mut community.channels {
            if ch.category_id.as_deref() == Some(&category_id) {
                ch.category_id = None;
            }
        }
    }
    Ok(())
}

/// Rename a channel category.
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
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::RenameCategory {
            category_id: category_id.clone(),
            new_name: new_name.clone(),
        },
    )
    .await?;

    // Optimistic local state update
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(cat) = community.categories.iter_mut().find(|c| c.id == category_id) {
            cat.name = new_name;
        }
    }
    Ok(())
}

/// Move a channel to a different category (or remove from any category).
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
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::MoveChannel {
            channel_id: channel_id.clone(),
            category_id: category_id.clone(),
        },
    )
    .await?;

    // Optimistic local state update
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
            ch.category_id = category_id;
        }
    }
    Ok(())
}

/// Reorder categories within a community.
#[tauri::command]
pub async fn reorder_categories(
    community_id: String,
    category_ids: Vec<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::ReorderCategories {
            category_ids: category_ids.clone(),
        },
    )
    .await?;

    // Optimistic local state update
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        for (i, cat_id) in category_ids.iter().enumerate() {
            if let Some(cat) = community.categories.iter_mut().find(|c| c.id == *cat_id) {
                cat.sort_order = i32::try_from(i).unwrap_or(i32::MAX);
            }
        }
        community.categories.sort_by_key(|c| c.sort_order);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Invite management
// ---------------------------------------------------------------------------

/// Create a community invite with embedded encrypted secrets.
///
/// Generates a 16-byte invite code, encrypts community secrets (slot_seed, MEK,
/// subkey_index, registry_key) using HKDF(code) → AES-256-GCM, and persists
/// the hashed code + encrypted blob to the DHT manifest. The raw code is
/// returned to the caller for building the invite link — it is never stored
/// in the DHT or broadcast over gossip.
#[tauri::command]
pub async fn create_community_invite(
    community_id: String,
    max_uses: Option<u32>,
    expires_in_seconds: Option<u64>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<InviteCreatedDto, String> {
    require_permission(state.inner(), &community_id, Permissions::CREATE_INSTANT_INVITE)?;

    // Generate 16-byte invite code (128 bits of entropy, 32 hex chars)
    let code = hex::encode(&rand_nonce()[..16]);
    let code_hash = rekindle_crypto::group::invite_crypto::hash_invite_code(&code);

    // Gather secrets from community state
    let (manifest_key, slot_seed, registry_key) = {
        let communities = state.communities.read();
        let cs = communities
            .get(&community_id)
            .ok_or("community not found")?;
        let mk = cs
            .manifest_key
            .clone()
            .ok_or("no manifest key for community")?;
        let ss = cs.slot_seed.clone().ok_or("no slot_seed available")?;
        let rk = cs
            .member_registry_key
            .clone()
            .ok_or("no registry key for community")?;
        (mk, ss, rk)
    };

    // Get MEK wire bytes from cache
    let mek_wire_b64 = {
        let cache = state.mek_cache.lock();
        let mek = cache.get(&community_id).ok_or("no MEK available")?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(mek.to_wire_bytes())
    };

    // Find next free subkey index(es) by reading the member registry
    let rc = state_helpers::routing_context(state.inner()).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let members =
        rekindle_protocol::dht::community::member_registry::read_member_index(&mgr, &registry_key)
            .await
            .map_err(|e| format!("read member index: {e}"))?;

    let occupied: std::collections::HashSet<u32> =
        members.iter().map(|m| m.subkey_index).collect();

    let effective_max_uses = max_uses.unwrap_or(1);
    let slots_needed = effective_max_uses;

    // Find contiguous or individual free slots up to slots_needed
    let mut free_slots: Vec<u32> = Vec::new();
    for idx in 0..rekindle_protocol::dht::community::member_registry::SLOTS_PER_SEGMENT {
        if !occupied.contains(&idx) {
            free_slots.push(idx);
            if free_slots.len() >= slots_needed as usize {
                break;
            }
        }
    }

    if free_slots.is_empty() {
        return Err("no free member slots available".into());
    }

    // Build InviteSecrets
    let secrets = rekindle_protocol::dht::community::InviteSecrets {
        slot_seed,
        mek_wire_bytes: mek_wire_b64,
        registry_key,
        assigned_subkey_index: if slots_needed == 1 {
            Some(free_slots[0])
        } else {
            None
        },
        slot_range: if slots_needed > 1 {
            Some((
                *free_slots.first().unwrap(),
                *free_slots.last().unwrap(),
            ))
        } else {
            None
        },
    };

    // Serialize and encrypt secrets
    let secrets_json = serde_json::to_vec(&secrets)
        .map_err(|e| format!("serialize invite secrets: {e}"))?;
    let encrypted =
        rekindle_crypto::group::invite_crypto::encrypt_invite_secrets(&code, &secrets_json)
            .map_err(|e| format!("encrypt invite secrets: {e}"))?;
    let encrypted_b64 = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&encrypted)
    };

    // Persist via execute_state_op (DHT write + gossip broadcast)
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::CreateInvite {
            code_hash: code_hash.clone(),
            max_uses,
            expires_in_seconds,
            encrypted_secrets: Some(encrypted_b64),
        },
    )
    .await?;

    // Persist the invite locally so admins can list codes from SQLite
    let owner_key = state_helpers::current_owner_key(state.inner()).unwrap_or_default();
    let cid = community_id.clone();
    let raw_code = code.clone();
    let ch = code_hash.clone();
    let now = i64::try_from(rekindle_utils::timestamp_secs()).unwrap_or(0);
    let mu = max_uses.map_or(0, i64::from);
    let exp = expires_in_seconds.map(|s| now + i64::try_from(s).unwrap_or(0));
    crate::db_helpers::db_fire(
        pool.inner(),
        "persist invite locally",
        move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO community_invites (owner_key, community_id, code, code_hash, max_uses, expires_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![owner_key, cid, raw_code, ch, mu, exp, now],
            )?;
            Ok(())
        },
    );

    Ok(InviteCreatedDto {
        code,
        manifest_key,
    })
}

/// Revoke a community invite by code hash.
#[tauri::command]
pub async fn revoke_community_invite(
    community_id: String,
    code_hash: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_COMMUNITY)?;
    let _ = pool;
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::RevokeInvite { code_hash },
    )
    .await
}

/// List active community invites from DHT manifest.
///
/// Merges raw invite codes from local SQLite (only available for invites
/// created by this node) so the frontend can build copyable invite links.
#[tauri::command]
pub async fn list_community_invites(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<InviteInfoDto>, String> {
    let rc = state_helpers::routing_context(state.inner()).ok_or("not attached")?;
    let manifest_key = manifest_key_for(state.inner(), &community_id)?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let invites =
        rekindle_protocol::dht::community::manifest::read_invites(&mgr, &manifest_key)
            .await
            .map_err(|e| format!("read invites: {e}"))?;

    // Fetch locally-stored raw codes from SQLite (only for invites this node created)
    let cid = community_id.clone();
    let local_codes: std::collections::HashMap<String, String> =
        crate::db_helpers::db_call_or_default(pool.inner(), move |conn| {
            let mut stmt = conn.prepare(
                "SELECT code_hash, code FROM community_invites WHERE community_id = ?",
            )?;
            let rows = stmt.query_map([&cid], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut map = std::collections::HashMap::new();
            for (hash, code) in rows.flatten() {
                map.insert(hash, code);
            }
            Ok(map)
        })
        .await;

    Ok(invites
        .into_iter()
        .map(|i| {
            let code = local_codes.get(&i.code_hash).cloned();
            InviteInfoDto {
                code_hash: i.code_hash,
                created_by: i.created_by,
                max_uses: if i.max_uses == 0 {
                    None
                } else {
                    Some(i.max_uses)
                },
                uses: i.use_count,
                expires_at: i.expires_at,
                created_at: i.created_at,
                code,
            }
        })
        .collect())
}

/// Send a message in a community channel.
///
/// Encrypts the message body with the community's MEK, then broadcasts a
/// `CommunityEnvelope::ChatMessage` to the gossip mesh via `send_to_mesh`.
/// Also persists the message to local SQLite.
#[tauri::command]
pub async fn send_channel_message(
    channel_id: String,
    body: String,
    reply_to_id: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let timestamp = db::timestamp_now();

    // --- Step 1: Find the community and get MEK + pseudonym ---
    let (community_id, mek_generation) = {
        let communities = state.communities.read();
        let community = communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .ok_or("channel not found in any community")?;
        (community.id.clone(), community.mek_generation)
    };

    require_permission(&state, &community_id, Permissions::SEND_MESSAGES)?;

    // Use pseudonym key as sender for channel messages (matches what the
    // server broadcasts to other members, keeping sender IDs consistent)
    let sender_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_else(|| owner_key.clone())
    };

    // --- Step 2: Encrypt with MEK ---
    let ciphertext = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(&community_id).ok_or_else(|| {
            "MEK not available — rejoin the community or wait for MEK delivery".to_string()
        })?;
        mek.encrypt(body.as_bytes())
            .map_err(|e| format!("MEK encryption failed: {e}"))?
    };

    // --- Step 3: Store plaintext in local SQLite FIRST (persist before send) ---
    let pool_for_queue = pool.inner().clone();
    let channel_id_clone = channel_id.clone();
    let sender_key_clone = sender_key.clone();
    let body_clone = body.clone();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        crate::message_repo::insert_channel_message(
            conn, &ok, &channel_id_clone, &sender_key_clone, &body_clone, timestamp, true,
            Some(mek_generation.cast_signed()),
        )
    })
    .await?;

    // --- Step 4: Send via gossip mesh (best-effort — message already persisted) ---
    let message_id = format!("msg_{}", hex::encode(rand_nonce().get(..8).unwrap_or(&[0; 8])));
    let lamport_ts = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.gossip.as_ref())
            .map_or(1, |g| g.lamport_counter + 1)
    };
    // Increment per-channel sequence number for gap detection
    let sequence = {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&community_id) {
            let s = cs.channel_sequences.entry(channel_id.clone()).or_insert(0);
            *s += 1;
            *s
        } else {
            1
        }
    };
    let chat_envelope = CommunityEnvelope::ChatMessage {
        channel_id: channel_id.clone(),
        message_id,
        author_pseudonym: sender_key.clone(),
        ciphertext: ciphertext.clone(),
        mek_generation,
        timestamp: timestamp.cast_unsigned(),
        reply_to_id,
        lamport_ts,
        sequence,
    };
    let delivery_result = send_to_mesh(state.inner(), &community_id, &chat_envelope);

    // Persist channel sequence to SQLite (non-blocking)
    {
        let ok_seq = state_helpers::current_owner_key(state.inner()).unwrap_or_default();
        let cid_seq = community_id.clone();
        let chid_seq = channel_id.clone();
        let seq_val = sequence;
        db_fire(pool.inner(), "persist channel sequence", move |conn| {
            conn.execute(
                "UPDATE channels SET my_sequence = ?1 WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
                rusqlite::params![seq_val.cast_signed(), ok_seq, cid_seq, chid_seq],
            )?;
            Ok(())
        });
    }

    // Layer 2: Write to SMPL channel record (member writes own subkey, non-blocking)
    {
        let (channel_key, my_subkey_index, slot_keypair_str) = {
            let communities = state.communities.read();
            match communities.get(&community_id) {
                Some(cs) => (
                    cs.channel_log_keys.get(&channel_id).cloned(),
                    cs.my_subkey_index,
                    cs.slot_keypair.clone(),
                ),
                None => (None, None, None),
            }
        };
        if let (Some(channel_key), Some(subkey_idx), Some(kp_str)) =
            (channel_key, my_subkey_index, slot_keypair_str)
        {
            let sender_key_for_log = sender_key.clone();
            let ciphertext_for_log = ciphertext.clone();
            let msg_id = if let CommunityEnvelope::ChatMessage { ref message_id, .. } = chat_envelope {
                Some(message_id.clone())
            } else {
                None
            };
            if let Some(rc) = state_helpers::routing_context(state.inner()) {
                tokio::spawn(async move {
                    if let Ok(kp) = kp_str.parse::<veilid_core::KeyPair>() {
                        let channel_msg = rekindle_protocol::dht::community::channel_record::ChannelMessage {
                            sequence,
                            sender_pseudonym: sender_key_for_log,
                            ciphertext: ciphertext_for_log,
                            mek_generation,
                            timestamp: timestamp.cast_unsigned(),
                            reply_to: None,
                            lamport_ts,
                            message_id: msg_id,
                        };
                        let mgr = rekindle_protocol::dht::DHTManager::new(rc);
                        if let Err(e) = rekindle_protocol::dht::community::channel_record::write_member_message(
                            &mgr, &channel_key, subkey_idx, kp, &channel_msg,
                        ).await {
                            tracing::debug!(error = %e, "SMPL channel write failed (non-fatal)");
                        }
                    }
                });
            }
        }
    }

    let delivery_status = if let Err(e) = delivery_result {
        tracing::warn!(error = %e, "server delivery failed — queuing for retry");
        queue_pending_channel_message(
            &state,
            &pool_for_queue,
            &community_id,
            &channel_id,
            &ciphertext,
            mek_generation,
            timestamp,
        );
        "queued"
    } else {
        "delivered"
    };

    // --- Step 5: Emit local echo to frontend ---
    let event = ChatEvent::MessageReceived {
        from: sender_key,
        body,
        timestamp: timestamp.cast_unsigned(),
        conversation_id: channel_id,
        server_message_id: None, // Local echo — message ID arrives via broadcast
        reply_to_id: None,       // Reply context not needed for local echo
    };
    let _ = app.emit("chat-event", &event);

    tracing::info!(status = delivery_status, "channel message sent");
    Ok(delivery_status.to_string())
}

/// Edit a previously sent channel message.
///
/// Re-encrypts the new body with the current MEK and sends an `EditMessage`
/// RPC to the community server.
#[tauri::command]
pub async fn edit_channel_message(
    channel_id: String,
    message_id: String,
    new_body: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool; // no longer needed for coordinator path
    let (community_id, mek_generation) = {
        let communities = state.communities.read();
        let community = communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .ok_or("channel not found in any community")?;
        (community.id.clone(), community.mek_generation)
    };

    let new_ciphertext = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(&community_id).ok_or("MEK not available")?;
        mek.encrypt(new_body.as_bytes())
            .map_err(|e| format!("MEK encryption failed: {e}"))?
    };

    // Edit messages propagate via gossip mesh (no coordinator needed).
    // Receivers validate that the sender is the original author locally.
    // We send the broadcast variant (MessageEdited) directly since there's no
    // coordinator intermediary to convert EditMessage → MessageEdited.
    send_to_mesh(
        state.inner(),
        &community_id,
        &rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::MessageEdited {
                channel_id,
                message_id,
                new_ciphertext,
                mek_generation,
                edited_at: rekindle_utils::timestamp_secs(),
            },
        ),
    )
}

/// Delete a channel message.
///
/// Sends via gossip mesh. Receivers check that the sender owns the message
/// or has `MANAGE_MESSAGES` permission locally.
#[tauri::command]
pub async fn delete_channel_message(
    channel_id: String,
    message_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool;
    let community_id = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .map(|c| c.id.clone())
            .ok_or("channel not found in any community")?
    };

    // Send the broadcast variant (MessageDeleted) directly via gossip.
    send_to_mesh(
        state.inner(),
        &community_id,
        &rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::MessageDeleted {
                channel_id,
                message_id,
            },
        ),
    )
}

/// Add a reaction to a community channel message.
///
/// Sent via gossip mesh — reactions are lightweight user actions that
/// don't require coordinator validation.
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
    // Send broadcast variant directly (ReactionAdded) since no coordinator intermediary.
    let reactor_pseudonym = {
        let communities = state.communities.read();
        communities.get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    send_to_mesh(
        state.inner(),
        &community_id,
        &rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::ReactionAdded {
                channel_id,
                message_id,
                emoji,
                reactor_pseudonym,
            },
        ),
    )
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
    // Send broadcast variant directly (ReactionRemoved) since no coordinator intermediary.
    let reactor_pseudonym = {
        let communities = state.communities.read();
        communities.get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    send_to_mesh(
        state.inner(),
        &community_id,
        &rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::ReactionRemoved {
                channel_id,
                message_id,
                emoji,
                reactor_pseudonym,
            },
        ),
    )
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
        communities.get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    send_to_mesh(
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
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::MessageUnpinned {
            channel_id,
            message_id,
        }),
    )
}

/// Get pinned messages for a community channel.
///
/// In the coordinator model, pins arrive via `MessagePinned` broadcasts and are
/// tracked in local state. This returns an empty list as a placeholder until
/// local pin tracking is implemented.
#[tauri::command]
pub async fn get_channel_pins(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<PinnedMessageInfoDto>, String> {
    require_permission(&state, &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id, channel_id, pinned_by, pinned_at FROM channel_pins \
             WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key, community_id, channel_id], |row| {
            Ok(PinnedMessageInfoDto {
                message_id: row.get(0)?,
                channel_id: row.get(1)?,
                pinned_by: row.get(2)?,
                pinned_at: row.get::<_, i64>(3).unwrap_or(0).cast_unsigned(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}

/// An audit log entry for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntryInfoDto {
    pub action: String,
    pub actor_pseudonym: String,
    pub target: Option<String>,
    pub details: Option<String>,
    pub timestamp: u64,
}

/// Get paginated audit log entries for a community.
#[tauri::command]
pub async fn get_audit_log(
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
    community_id: String,
    _before_timestamp: Option<u64>,
    limit: u32,
) -> Result<Vec<AuditLogEntryInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_AUDIT_LOG)?;

    // Get audit record key from coordinator service
    let audit_key = {
        let services = state.coordinator_services.read();
        services.get(&community_id).and_then(|h| {
            let logger = h.state_mgr.audit_logger();
            let guard = logger.lock();
            guard.record_key().map(String::from)
        })
    };

    let audit_key = if let Some(k) = audit_key {
        k
    } else {
        // Try reading from manifest subkey 14
        let rc = state_helpers::routing_context(&state).ok_or("not attached")?;
        let mgr = rekindle_protocol::dht::DHTManager::new(rc);
        let manifest_key = {
            let communities = state.communities.read();
            communities
                .get(&community_id)
                .and_then(|c| c.manifest_key.clone().or_else(|| Some(c.id.clone())))
                .ok_or("community not found")?
        };
        rekindle_protocol::dht::community::manifest::read_audit_log_key(&mgr, &manifest_key)
            .await
            .map_err(|e| format!("read audit key: {e}"))?
            .ok_or("no audit log configured")?
    };

    let capped_limit = limit.min(100) as usize;
    let entries = services::coordinator::audit::read_entries(&state, &audit_key, capped_limit)
        .await?;

    Ok(entries
        .into_iter()
        .map(|e| AuditLogEntryInfoDto {
            action: format!("{:?}", e.action),
            actor_pseudonym: e.actor_pseudonym,
            target: Some(format!("{:?}", e.target)),
            details: e.reason,
            timestamp: e.timestamp,
        })
        .collect())
}

/// Event info DTO re-exported from the channel module.
pub use crate::channels::community_channel::EventInfoDto;
/// Event RSVP DTO re-exported from the channel module.
pub use crate::channels::community_channel::EventRsvpInfoDto;

/// Create a community event.
#[tauri::command]
pub async fn create_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    title: String,
    description: String,
    start_time: u64,
    end_time: Option<u64>,
    channel_id: Option<String>,
    max_attendees: Option<u32>,
) -> Result<String, String> {
    let _ = pool;
    // Generate event ID locally for optimistic UI; coordinator assigns canonical ID via broadcast
    let event_id = format!("evt_{}", hex::encode(&rand_nonce()[..8]));

    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::CreateEvent {
            title,
            description,
            start_time,
            end_time,
            channel_id,
            max_attendees,
        }),
    )?;

    Ok(event_id)
}

/// Edit a community event.
#[tauri::command]
pub async fn edit_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
    title: Option<String>,
    description: Option<String>,
    start_time: Option<u64>,
    end_time: Option<u64>,
    channel_id: Option<String>,
    max_attendees: Option<u32>,
) -> Result<(), String> {
    let _ = pool;
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EditEvent {
            event_id,
            title,
            description,
            start_time,
            end_time,
            channel_id,
            max_attendees,
        }),
    )
}

/// Delete a community event.
#[tauri::command]
pub async fn delete_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    let _ = pool;
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::DeleteEvent { event_id }),
    )
}

/// Cancel a community event (sets status to "canceled").
#[tauri::command]
pub async fn cancel_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    let _ = pool;
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::CancelEvent { event_id }),
    )
}

/// RSVP to a community event.
#[tauri::command]
pub async fn rsvp_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
    status: String,
) -> Result<(), String> {
    let _ = pool;
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::RsvpEvent { event_id, status }),
    )
}

/// Get community events.
///
/// In the coordinator model, events arrive via `EventCreated` broadcasts and are
/// tracked in local state. Returns an empty list as a placeholder until local
/// event tracking is implemented.
#[tauri::command]
pub async fn get_events(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<Vec<EventInfoDto>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, title, description, creator_pseudonym, start_time, end_time, \
                    channel_id, max_attendees, created_at, status \
             FROM community_events \
             WHERE owner_key = ?1 AND community_id = ?2 \
             ORDER BY start_time ASC",
        )?;
        let events: Vec<EventInfoDto> = stmt
            .query_map(rusqlite::params![owner_key, community_id], |row| {
                Ok(EventInfoDto {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    creator_pseudonym: row.get(3)?,
                    start_time: row.get::<_, i64>(4).unwrap_or(0).cast_unsigned(),
                    end_time: row.get::<_, Option<i64>>(5)?.map(i64::cast_unsigned),
                    channel_id: row.get(6)?,
                    max_attendees: row.get::<_, Option<i32>>(7)?.map(i32::cast_unsigned),
                    created_at: row.get::<_, i64>(8).unwrap_or(0).cast_unsigned(),
                    status: row.get(9)?,
                    rsvps: Vec::new(), // filled below
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Fill RSVPs for each event
        let mut rsvp_stmt = conn.prepare(
            "SELECT pseudonym_key, status FROM event_rsvps \
             WHERE owner_key = ?1 AND community_id = ?2 AND event_id = ?3",
        )?;
        let events_with_rsvps = events
            .into_iter()
            .map(|mut evt| {
                if let Ok(rsvps) = rsvp_stmt.query_map(
                    rusqlite::params![owner_key, community_id, evt.id],
                    |row| {
                        Ok(EventRsvpInfoDto {
                            pseudonym_key: row.get(0)?,
                            status: row.get(1)?,
                        })
                    },
                ) {
                    evt.rsvps = rsvps.filter_map(Result::ok).collect();
                }
                evt
            })
            .collect();

        Ok(events_with_rsvps)
    })
    .await
}

// ---------------------------------------------------------------------------
// Thread commands
// ---------------------------------------------------------------------------

/// Thread info DTO re-exported from the channel module.
pub use crate::channels::community_channel::ThreadInfoDto;

/// Create a thread from a message in a channel.
#[tauri::command]
pub async fn create_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    name: String,
    starter_message_id: String,
) -> Result<String, String> {
    let _ = pool;
    // Generate thread ID locally for optimistic UI; coordinator assigns canonical ID via broadcast
    let thread_id = format!("thr_{}", hex::encode(&rand_nonce()[..8]));

    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::CreateThread {
            channel_id,
            name,
            starter_message_id,
        }),
    )?;

    Ok(thread_id)
}

/// Get threads for a channel.
///
/// In the coordinator model, threads arrive via `ThreadCreated` broadcasts and
/// are tracked in local state. Returns an empty list as a placeholder until
/// local thread tracking is implemented.
#[tauri::command]
pub async fn get_channel_threads(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<ThreadInfoDto>, String> {
    require_permission(&state, &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, channel_id, name, starter_message_id, creator_pseudonym, \
                    created_at, archived, auto_archive_seconds, last_message_at, message_count \
             FROM community_threads \
             WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ?3 \
             ORDER BY last_message_at DESC",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![owner_key, community_id, channel_id],
            |row| {
                Ok(ThreadInfoDto {
                    id: row.get(0)?,
                    channel_id: row.get(1)?,
                    name: row.get(2)?,
                    starter_message_id: row.get(3)?,
                    creator_pseudonym: row.get(4)?,
                    created_at: row.get::<_, i64>(5).unwrap_or(0).cast_unsigned(),
                    archived: row.get::<_, i32>(6).unwrap_or(0) != 0,
                    auto_archive_seconds: row.get::<_, i32>(7).unwrap_or(0).cast_unsigned(),
                    last_message_at: row.get::<_, i64>(8).unwrap_or(0).cast_unsigned(),
                    message_count: row.get::<_, i32>(9).unwrap_or(0).cast_unsigned(),
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}

/// Send a message to a thread (encrypted with MEK).
#[tauri::command]
pub async fn send_thread_message(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
    body: String,
) -> Result<(), String> {
    let _ = pool;
    // Encrypt with MEK (same pattern as send_channel_message)
    let (ciphertext, mek_generation) = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(&community_id).ok_or_else(|| {
            "MEK not available — rejoin the community or wait for MEK delivery".to_string()
        })?;
        let ct = mek
            .encrypt(body.as_bytes())
            .map_err(|e| format!("MEK encryption failed: {e}"))?;
        (ct, mek.generation())
    };

    // Thread messages are chat messages — send via gossip mesh, not coordinator.
    // Send broadcast variant directly (ThreadMessageReceived) since no coordinator intermediary.
    let sender_pseudonym = {
        let communities = state.communities.read();
        communities.get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let message_id = format!("tmsg_{}", hex::encode(&rand_nonce()[..8]));
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::ThreadMessageReceived {
            thread_id,
            message_id,
            sender_pseudonym,
            ciphertext,
            mek_generation,
            timestamp: rekindle_utils::timestamp_secs(),
            reply_to_id: None,
        }),
    )
}

/// Get thread message history (decrypted with MEK).
///
/// In the coordinator model, thread messages arrive via broadcasts and are
/// tracked in local state. Returns an empty list as a placeholder until
/// local thread message tracking is implemented.
#[tauri::command]
pub async fn get_thread_messages(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
    limit: u32,
    before_timestamp: Option<u64>,
) -> Result<Vec<Message>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let lim = i64::from(limit.min(200));
    db_call(pool.inner(), move |conn| {
        let before_ts = before_timestamp.map_or(i64::MAX, u64::cast_signed);
        let mut stmt = conn.prepare(
            "SELECT message_id, sender_pseudonym, body, timestamp, reply_to_id \
             FROM thread_messages \
             WHERE owner_key = ?1 AND community_id = ?2 AND thread_id = ?3 AND timestamp < ?4 \
             ORDER BY timestamp DESC LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![owner_key, community_id, thread_id, before_ts, lim],
            |row| {
                let sender: String = row.get(1)?;
                let is_own = sender == my_pseudonym;
                Ok(Message {
                    id: 0, // thread messages don't use auto-increment id
                    sender_id: sender,
                    body: row.get(2)?,
                    timestamp: row.get(3)?,
                    is_own,
                    server_message_id: row.get(0)?,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}

/// Archive a thread.
#[tauri::command]
pub async fn archive_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
) -> Result<(), String> {
    let _ = pool;
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::ArchiveThread { thread_id }),
    )
}

/// Unarchive a thread.
#[tauri::command]
pub async fn unarchive_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
) -> Result<(), String> {
    let _ = pool;
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::UnarchiveThread { thread_id }),
    )
}

// ---------------------------------------------------------------------------
// Game server favorites
// ---------------------------------------------------------------------------

/// Game server info DTO re-exported from the channel module.
pub use crate::channels::community_channel::GameServerInfoDto;

/// Add a game server to a community's favorites.
#[tauri::command]
pub async fn add_game_server(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    game_id: String,
    label: String,
    address: String,
) -> Result<String, String> {
    let _ = pool;
    // Generate game server ID locally — the locally-generated ID is canonical
    let server_id = format!("gs_{}", hex::encode(&rand_nonce()[..8]));

    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::AddGameServer { game_id, label, address }),
    )?;

    Ok(server_id)
}

/// Remove a game server from a community's favorites.
#[tauri::command]
pub async fn remove_game_server(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    server_id: String,
) -> Result<(), String> {
    let _ = pool;
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::RemoveGameServer { server_id }),
    )
}

/// Get all game servers for a community.
///
/// In the coordinator model, game servers arrive via `GameServerAdded` broadcasts
/// and are tracked in local state. Returns an empty list as a placeholder until
/// local game server tracking is implemented.
#[tauri::command]
pub async fn get_game_servers(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<Vec<GameServerInfoDto>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, game_id, label, address, added_by, created_at FROM game_servers \
             WHERE owner_key = ?1 AND community_id = ?2 \
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key, community_id], |row| {
            Ok(GameServerInfoDto {
                id: row.get(0)?,
                game_id: row.get(1)?,
                label: row.get(2)?,
                address: row.get(3)?,
                added_by: row.get(4)?,
                created_at: row.get::<_, i64>(5).unwrap_or(0).cast_unsigned(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}

/// Pending channel message queued for retry delivery to the community server.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingChannelMessage {
    pub community_id: String,
    pub channel_id: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: i64,
}

/// Queue a failed channel message for retry via `pending_messages` table.
///
/// Serializes as JSON into the `body` column. Uses `community_id` as `recipient_key`
/// so `sync_service` can distinguish channel retries from DM retries.
fn queue_pending_channel_message(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    ciphertext: &[u8],
    mek_generation: u64,
    timestamp: i64,
) {
    let pending = PendingChannelMessage {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        ciphertext: ciphertext.to_vec(),
        mek_generation,
        timestamp,
    };
    let body = match serde_json::to_string(&pending) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize pending channel message");
            return;
        }
    };

    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let recipient = community_id.to_string();
    let now = crate::db::timestamp_now();
    db_fire(pool, "queue pending channel message", move |conn| {
        conn.execute(
            "INSERT INTO pending_messages (owner_key, recipient_key, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![owner_key, recipient, body, now],
        )?;
        Ok(())
    });
}

pub(crate) fn rand_nonce() -> Vec<u8> {
    use rand::RngCore;
    let mut nonce = vec![0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Execute a Tier 2 state operation. If we hold the manifest keypair, write
/// directly to DHT + gossip broadcast. Otherwise broadcast via gossip mesh —
/// any admin/coordinator with the keypair will pick it up and persist to DHT.
pub(crate) async fn execute_state_op(
    state: &SharedState,
    community_id: &str,
    payload: ControlPayload,
) -> Result<(), String> {
    let has_manifest_kp = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.manifest_owner_keypair.as_ref())
            .is_some()
    };

    if has_manifest_kp {
        // Admin path: persist to DHT directly + broadcast via gossip mesh
        persist_control_to_dht(state, community_id, &payload).await?;
        send_to_mesh(
            state,
            community_id,
            &CommunityEnvelope::Control(payload),
        )
    } else {
        // Non-admin path: send to coordinator via app_call for confirmed delivery,
        // then gossip broadcast for fast propagation to all members.
        let envelope = CommunityEnvelope::Control(payload.clone());
        let signed_bytes = sign_control_for_coordinator(state, community_id, &envelope)?;

        match send_to_coordinator_confirmed(state, community_id, &signed_bytes).await {
            Ok(_ack) => {
                // Coordinator received it — also gossip broadcast so all peers learn immediately
                send_to_mesh(
                    state,
                    community_id,
                    &CommunityEnvelope::Control(payload),
                )
            }
            Err(e) => Err(format!("coordinator unreachable for state op: {e}")),
        }
    }
}

/// Sign and serialize a community envelope for sending to the coordinator.
fn sign_control_for_coordinator(
    state: &SharedState,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<Vec<u8>, String> {
    let envelope_bytes =
        serde_json::to_vec(envelope).map_err(|e| format!("serialize envelope: {e}"))?;

    let (pseudonym_key, signing_key) = {
        let communities = state.communities.read();
        let cs = communities
            .get(community_id)
            .ok_or("community not found")?;
        let pk = cs
            .my_pseudonym_key
            .clone()
            .ok_or("no pseudonym key")?;
        drop(communities);

        let secret = state.identity_secret.lock();
        let secret = secret.ok_or("no identity secret")?;
        let sk = rekindle_crypto::group::pseudonym::derive_community_pseudonym(
            &secret,
            community_id,
        );
        (pk, sk)
    };

    let signed = rekindle_protocol::dht::community::envelope::sign_envelope(
        &signing_key,
        community_id,
        &pseudonym_key,
        &envelope_bytes,
    );
    serde_json::to_vec(&signed).map_err(|e| format!("serialize signed: {e}"))
}

/// Send a signed payload to the coordinator via app_call with retry.
/// Returns the response bytes on success, or error after 3 attempts.
async fn send_to_coordinator_confirmed(
    state: &SharedState,
    community_id: &str,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    let route_blob = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.coordinator_route_blob.clone())
            .ok_or("no coordinator route — coordinator may be offline")?
    };
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;

    for attempt in 0..3u32 {
        let route_id = match rc.api().import_remote_private_route(route_blob.clone()) {
            Ok(id) => id,
            Err(e) => {
                if attempt < 2 {
                    tracing::warn!(attempt, error = %e, "app_call route import failed, retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(1u64 << attempt)).await;
                    continue;
                }
                return Err(format!("route import failed after 3 attempts: {e}"));
            }
        };
        match rc
            .app_call(veilid_core::Target::RouteId(route_id), payload.to_vec())
            .await
        {
            Ok(response) => return Ok(response),
            Err(e) if attempt < 2 => {
                tracing::warn!(attempt, error = %e, "app_call to coordinator failed, retrying");
                tokio::time::sleep(std::time::Duration::from_secs(1u64 << attempt)).await;
            }
            Err(e) => return Err(format!("app_call failed after 3 attempts: {e}")),
        }
    }
    Err("app_call failed: max retries exceeded".into())
}

/// Persist a Tier 2 control payload directly to the DHT manifest.
///
/// Called by `execute_state_op` when the local node holds the manifest keypair.
/// Mirrors the logic in `coordinator::state_manager::persist_control_to_manifest`.
async fn persist_control_to_dht(
    state: &SharedState,
    community_id: &str,
    payload: &ControlPayload,
) -> Result<(), String> {
    use rekindle_protocol::dht::DHTManager;

    let (manifest_key, kp_str) = {
        let c = state.communities.read();
        let cs = c.get(community_id).ok_or("community not found")?;
        (
            cs.manifest_key.clone().ok_or("no manifest key")?,
            cs.manifest_owner_keypair
                .clone()
                .ok_or("no manifest keypair")?,
        )
    };

    let rc = state_helpers::routing_context(state).ok_or("Veilid network not attached")?;
    let kp: veilid_core::KeyPair = kp_str.parse().map_err(|e| format!("parse keypair: {e}"))?;
    let dht = DHTManager::new(rc).with_writer(kp.clone());

    dht.open_record_writable(&manifest_key, kp)
        .await
        .map_err(|e| format!("open manifest: {e}"))?;

    apply_control_to_manifest(&dht, &manifest_key, payload).await
}

/// Apply a control payload mutation to the DHT manifest record.
///
/// Extracted from `persist_control_to_dht` for clippy line limits.
async fn apply_control_to_manifest(
    dht: &rekindle_protocol::dht::DHTManager,
    manifest_key: &str,
    payload: &ControlPayload,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::manifest;
    use rekindle_protocol::dht::community::types::{
        CategoryEntry, ChannelEntryV2, ChannelKind, RoleEntryV2,
    };

    match payload {
        // ── Channels ──
        ControlPayload::CreateChannel {
            name,
            channel_type,
            category_id,
            channel_id,
        } => {
            let mut chs = manifest::read_channels(dht, manifest_key)
                .await
                .unwrap_or_default();
            let sort_order = u16::try_from(chs.len()).unwrap_or(u16::MAX);
            let kind = channel_type
                .parse::<ChannelKind>()
                .unwrap_or(ChannelKind::Text);
            chs.push(ChannelEntryV2 {
                id: channel_id.clone(),
                name: name.clone(),
                kind,
                sort_order,
                category_id: category_id.clone(),
                topic: String::new(),
                slowmode_seconds: 0,
                nsfw: false,
                message_record_key: None,
                mek_generation: 0,
                permission_overwrites: vec![],
                log_key: None,
            });
            manifest::write_channels(dht, manifest_key, &chs)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::DeleteChannel { channel_id } => {
            let mut chs = manifest::read_channels(dht, manifest_key)
                .await
                .unwrap_or_default();
            chs.retain(|c| c.id != *channel_id);
            manifest::write_channels(dht, manifest_key, &chs)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::RenameChannel {
            channel_id,
            new_name,
        } => {
            let mut chs = manifest::read_channels(dht, manifest_key)
                .await
                .unwrap_or_default();
            if let Some(ch) = chs.iter_mut().find(|c| c.id == *channel_id) {
                ch.name.clone_from(new_name);
            }
            manifest::write_channels(dht, manifest_key, &chs)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::SetChannelTopic { channel_id, topic } => {
            let mut chs = manifest::read_channels(dht, manifest_key)
                .await
                .unwrap_or_default();
            if let Some(ch) = chs.iter_mut().find(|c| c.id == *channel_id) {
                ch.topic.clone_from(topic);
            }
            manifest::write_channels(dht, manifest_key, &chs)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::ReorderChannels { channel_ids } => {
            let mut chs = manifest::read_channels(dht, manifest_key)
                .await
                .unwrap_or_default();
            for (i, id) in channel_ids.iter().enumerate() {
                if let Some(ch) = chs.iter_mut().find(|c| c.id == *id) {
                    ch.sort_order = u16::try_from(i).unwrap_or(u16::MAX);
                }
            }
            manifest::write_channels(dht, manifest_key, &chs)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::SetSlowmode {
            channel_id,
            seconds,
        } => {
            let mut chs = manifest::read_channels(dht, manifest_key)
                .await
                .unwrap_or_default();
            if let Some(ch) = chs.iter_mut().find(|c| c.id == *channel_id) {
                ch.slowmode_seconds = *seconds;
            }
            manifest::write_channels(dht, manifest_key, &chs)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::MoveChannel {
            channel_id,
            category_id,
        } => {
            let mut chs = manifest::read_channels(dht, manifest_key)
                .await
                .unwrap_or_default();
            if let Some(ch) = chs.iter_mut().find(|c| c.id == *channel_id) {
                ch.category_id.clone_from(category_id);
            }
            manifest::write_channels(dht, manifest_key, &chs)
                .await
                .map_err(|e| format!("{e}"))
        }

        // ── Categories ──
        ControlPayload::CreateCategory { name } => {
            let mut cats = manifest::read_categories(dht, manifest_key)
                .await
                .unwrap_or_default();
            let sort = i32::try_from(cats.len()).unwrap_or(i32::MAX);
            cats.push(CategoryEntry {
                id: format!("cat_{}", hex::encode(&rand_nonce()[..8])),
                name: name.clone(),
                sort_order: sort,
            });
            manifest::write_categories(dht, manifest_key, &cats)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::DeleteCategory { category_id } => {
            let mut cats = manifest::read_categories(dht, manifest_key)
                .await
                .unwrap_or_default();
            cats.retain(|c| c.id != *category_id);
            manifest::write_categories(dht, manifest_key, &cats)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::RenameCategory {
            category_id,
            new_name,
        } => {
            let mut cats = manifest::read_categories(dht, manifest_key)
                .await
                .unwrap_or_default();
            if let Some(cat) = cats.iter_mut().find(|c| c.id == *category_id) {
                cat.name.clone_from(new_name);
            }
            manifest::write_categories(dht, manifest_key, &cats)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::ReorderCategories { category_ids } => {
            let mut cats = manifest::read_categories(dht, manifest_key)
                .await
                .unwrap_or_default();
            for (i, id) in category_ids.iter().enumerate() {
                if let Some(cat) = cats.iter_mut().find(|c| c.id == *id) {
                    cat.sort_order = i32::try_from(i).unwrap_or(i32::MAX);
                }
            }
            manifest::write_categories(dht, manifest_key, &cats)
                .await
                .map_err(|e| format!("{e}"))
        }

        // ── Roles ──
        ControlPayload::CreateRole {
            name,
            color,
            permissions,
            hoist,
            mentionable,
        } => {
            let mut roles = manifest::read_roles(dht, manifest_key)
                .await
                .unwrap_or_default();
            let next_id = roles.iter().map(|r| r.id).max().unwrap_or(4) + 1;
            let position = i32::try_from(roles.len()).unwrap_or(i32::MAX);
            roles.push(RoleEntryV2 {
                id: next_id,
                name: name.clone(),
                color: *color,
                permissions: *permissions,
                position,
                hoist: *hoist,
                mentionable: *mentionable,
            });
            manifest::write_roles(dht, manifest_key, &roles)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::DeleteRole { role_id } => {
            let mut roles = manifest::read_roles(dht, manifest_key)
                .await
                .unwrap_or_default();
            roles.retain(|r| r.id != *role_id);
            manifest::write_roles(dht, manifest_key, &roles)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::EditRole {
            role_id,
            name,
            color,
            permissions,
            position,
            hoist,
            mentionable,
        } => {
            let mut roles = manifest::read_roles(dht, manifest_key)
                .await
                .unwrap_or_default();
            if let Some(r) = roles.iter_mut().find(|r| r.id == *role_id) {
                if let Some(ref n) = name {
                    r.name.clone_from(n);
                }
                if let Some(c) = color {
                    r.color = *c;
                }
                if let Some(p) = permissions {
                    r.permissions = *p;
                }
                if let Some(pos) = position {
                    r.position = *pos;
                }
                if let Some(h) = hoist {
                    r.hoist = *h;
                }
                if let Some(m) = mentionable {
                    r.mentionable = *m;
                }
            }
            manifest::write_roles(dht, manifest_key, &roles)
                .await
                .map_err(|e| format!("{e}"))
        }

        // Bans, invites, metadata, and non-manifest payloads — split out for clippy line limit
        other => apply_control_to_manifest_ext(dht, manifest_key, other).await,
    }
}

/// Extended manifest control payload handling (bans, invites, metadata).
///
/// Split from `apply_control_to_manifest` for clippy line limits.
async fn apply_control_to_manifest_ext(
    dht: &rekindle_protocol::dht::DHTManager,
    manifest_key: &str,
    payload: &ControlPayload,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::manifest;
    use rekindle_protocol::dht::community::types::BanEntry;

    match payload {
        // ── Bans ──
        ControlPayload::Ban {
            target_pseudonym, ..
        } => {
            let mut bans = manifest::read_bans(dht, manifest_key)
                .await
                .unwrap_or_default();
            bans.push(BanEntry {
                pseudonym_key: target_pseudonym.clone(),
                reason: None,
                banned_by: String::new(),
                banned_at: rekindle_utils::timestamp_secs(),
            });
            manifest::write_bans(dht, manifest_key, &bans)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::Unban {
            target_pseudonym, ..
        } => {
            let mut bans = manifest::read_bans(dht, manifest_key)
                .await
                .unwrap_or_default();
            bans.retain(|b| b.pseudonym_key != *target_pseudonym);
            manifest::write_bans(dht, manifest_key, &bans)
                .await
                .map_err(|e| format!("{e}"))
        }

        // ── Metadata ──
        ControlPayload::UpdateCommunity { name, description } => {
            let mut meta = manifest::read_metadata(dht, manifest_key)
                .await
                .ok()
                .flatten()
                .unwrap_or(
                    rekindle_protocol::dht::community::types::CommunityMetadataV2 {
                        name: String::new(),
                        description: None,
                        icon_hash: None,
                        created_at: 0,
                        owner_pseudonym: String::new(),
                        last_refreshed: 0,
                    },
                );
            if let Some(ref n) = name {
                meta.name.clone_from(n);
            }
            if let Some(ref d) = description {
                meta.description = Some(d.clone());
            }
            manifest::write_metadata(dht, manifest_key, &meta)
                .await
                .map_err(|e| format!("{e}"))
        }

        // ── Invites ──
        ControlPayload::CreateInvite {
            code_hash,
            max_uses,
            expires_in_seconds,
            encrypted_secrets,
        } => {
            use rekindle_protocol::dht::community::types::InviteEntry;
            let now = rekindle_utils::timestamp_secs();
            let expires_at = expires_in_seconds.map(|s| now + s);
            let mut invites = manifest::read_invites(dht, manifest_key)
                .await
                .unwrap_or_default();
            // Prune expired invites
            invites.retain(|inv| inv.expires_at.is_none_or(|exp| exp > now));
            invites.push(InviteEntry {
                code_hash: code_hash.clone(),
                created_by: String::new(), // filled by caller if needed
                created_at: now,
                expires_at,
                max_uses: max_uses.unwrap_or(0),
                use_count: 0,
                encrypted_secrets: encrypted_secrets.clone(),
            });
            manifest::write_invites(dht, manifest_key, &invites)
                .await
                .map_err(|e| format!("{e}"))
        }
        ControlPayload::RevokeInvite { code_hash } => {
            let mut invites = manifest::read_invites(dht, manifest_key)
                .await
                .unwrap_or_default();
            invites.retain(|inv| inv.code_hash != *code_hash);
            manifest::write_invites(dht, manifest_key, &invites)
                .await
                .map_err(|e| format!("{e}"))
        }

        // Non-manifest payloads don't need DHT persistence
        _ => Ok(()),
    }
}

/// Send a community envelope via the gossip mesh (peer-to-peer).
///
/// Used for ChatMessage, TypingIndicator, PresenceUpdate, and all Tier 1
/// operations (pins, events, threads, game servers, reactions).
/// Tier 2 operations use `execute_state_op()` which combines this with DHT writes.
///
/// Signs the envelope with our pseudonym key, inserts into the dedup cache,
/// increments the Lamport counter, and sends to D gossip peers.
pub(crate) fn send_to_mesh(
    state: &SharedState,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope;

    // Get our pseudonym key
    let my_pseudonym_key = {
        let communities = state.communities.read();
        let c = communities
            .get(community_id)
            .ok_or("community not found")?;
        c.my_pseudonym_key.clone().unwrap_or_default()
    };

    // Sign envelope with pseudonym signing key
    let signing_key = {
        let secret = state.identity_secret.lock();
        let s = (*secret).ok_or("identity not unlocked")?;
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&s, community_id)
    };
    let envelope_bytes =
        serde_json::to_vec(envelope).map_err(|e| format!("serialize envelope: {e}"))?;
    let signed = envelope::sign_envelope(
        &signing_key,
        community_id,
        &my_pseudonym_key,
        &envelope_bytes,
    );

    // Insert into dedup cache so we don't process our own gossip forward
    let dedup_key = extract_mesh_dedup_key(envelope);
    state
        .dedup_cache
        .lock()
        .check_and_insert(community_id, &my_pseudonym_key, &dedup_key);

    // Increment lamport counter
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            if let Some(ref mut gossip) = cs.gossip {
                gossip.lamport_counter += 1;
            }
        }
    }

    // Send to D gossip peers
    send_to_mesh_raw(state, community_id, &signed);

    Ok(())
}

/// Low-level: send signed envelope bytes to D gossip peers.
///
/// Called by both `send_to_mesh()` (originator) and `broadcast_via_gossip()` (coordinator).
pub(crate) fn send_to_mesh_raw(
    state: &SharedState,
    community_id: &str,
    signed: &rekindle_protocol::dht::community::envelope::SignedEnvelope,
) {
    let Ok(signed_bytes) = serde_json::to_vec(signed) else {
        tracing::warn!(community = %community_id, "send_to_mesh_raw: failed to serialize envelope");
        return;
    };

    let Some(rc) = state_helpers::routing_context(state) else {
        tracing::warn!(community = %community_id, "send_to_mesh_raw: no routing context");
        return;
    };

    let peers: Vec<(String, Vec<u8>)> = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else {
            tracing::warn!(community = %community_id, "send_to_mesh_raw: community not found");
            return;
        };
        let Some(ref gossip) = cs.gossip else {
            tracing::warn!(community = %community_id, "send_to_mesh_raw: gossip overlay is None");
            return;
        };
        if gossip.peers.is_empty() {
            tracing::warn!(community = %community_id, "send_to_mesh_raw: no gossip peers — message will not be delivered");
            return;
        }
        gossip.peers.iter().map(|(k, m)| (k.clone(), m.route_blob.clone())).collect()
    };

    tracing::info!(
        community = %community_id,
        peer_count = peers.len(),
        "send_to_mesh_raw: sending to {} peers",
        peers.len(),
    );

    // Extract message_id from the envelope for delivery tracking
    let message_id: Option<String> = serde_json::from_slice::<rekindle_protocol::dht::community::envelope::CommunityEnvelope>(
        &signed.envelope_bytes,
    )
    .ok()
    .and_then(|env| {
        if let rekindle_protocol::dht::community::envelope::CommunityEnvelope::ChatMessage { message_id, .. } = env {
            Some(message_id)
        } else {
            None
        }
    });

    for (peer_key, route_blob) in peers {
        let rc = rc.clone();
        let data = signed_bytes.clone();
        let cid = community_id.to_string();
        let state_clone = state.clone();
        let msg_id = message_id.clone();
        let pk = peer_key.clone();
        tokio::spawn(async move {
            // Try primary route blob — use app_call for confirmed delivery (Xfire ACK pattern).
            let send_result = match rc.api().import_remote_private_route(route_blob) {
                Ok(route_id) => {
                    match rc.app_call(veilid_core::Target::RouteId(route_id), data.clone()).await {
                        Ok(_ack) => Ok(()),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(veilid_core::VeilidAPIError::generic(e)),
            };

            if send_result.is_ok() {
                // Delivery confirmed — record in SQLite
                if let Some(ref mid) = msg_id {
                    record_delivery(&state_clone, mid, &cid, &pk, "delivered");
                }
                return;
            }

            // Primary failed — attempt DHT re-resolution
            tracing::info!(community = %cid, peer = %pk, "route stale, attempting DHT re-resolve");
            let fresh_blob = resolve_peer_route_from_db(&state_clone, &cid, &pk).await;
            if let Some(blob) = fresh_blob {
                match rc.api().import_remote_private_route(blob.clone()) {
                    Ok(route_id) => {
                        match rc.app_call(veilid_core::Target::RouteId(route_id), data).await {
                            Ok(_ack) => {
                                update_peer_route(&state_clone, &cid, &pk, blob);
                                if let Some(ref mid) = msg_id {
                                    record_delivery(&state_clone, mid, &cid, &pk, "delivered");
                                }
                            }
                            Err(e) => {
                                tracing::warn!(community = %cid, peer = %pk, error = %e, "re-resolved route: app_call still failed");
                                if let Some(ref mid) = msg_id {
                                    record_delivery(&state_clone, mid, &cid, &pk, "failed");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(community = %cid, peer = %pk, error = %e, "re-resolved route also invalid");
                        if let Some(ref mid) = msg_id {
                            record_delivery(&state_clone, mid, &cid, &pk, "failed");
                        }
                    }
                }
            } else {
                tracing::warn!(community = %cid, peer = %pk, "no fresh route found in DHT");
                if let Some(ref mid) = msg_id {
                    record_delivery(&state_clone, mid, &cid, &pk, "failed");
                }
            }
        });
    }
}

/// Record a message delivery status to SQLite (non-blocking).
fn record_delivery(state: &SharedState, message_id: &str, community_id: &str, recipient: &str, status: &str) {
    use tauri::Manager as _;
    let app_handle = state.app_handle.read().clone();
    if let Some(ref ah) = app_handle {
        if let Some(pool) = ah.try_state::<crate::db::DbPool>() {
            let mid = message_id.to_string();
            let cid = community_id.to_string();
            let rp = recipient.to_string();
            let st = status.to_string();
            let now = rekindle_utils::timestamp_secs();
            crate::db_helpers::db_fire(&pool, "record_delivery", move |conn| {
                conn.execute(
                    "INSERT INTO message_delivery (message_id, community_id, recipient_pseudonym, status, attempts, last_attempt_at) \
                     VALUES (?1, ?2, ?3, ?4, 1, ?5) \
                     ON CONFLICT(message_id, recipient_pseudonym) \
                     DO UPDATE SET status=excluded.status, attempts=attempts+1, last_attempt_at=excluded.last_attempt_at",
                    rusqlite::params![mid, cid, rp, st, now.cast_signed()],
                )?;
                Ok(())
            });
        }
    }
}

/// Re-resolve a peer's route blob from the SMPL member registry via DHT.
/// Looks up the peer's subkey_index from SQLite, then reads their presence
/// from DHT with force_refresh to get the latest route_blob.
async fn resolve_peer_route_from_db(
    state: &SharedState,
    community_id: &str,
    peer_pseudonym: &str,
) -> Option<Vec<u8>> {
    use rekindle_protocol::dht::community::member_registry;

    // Look up registry_key from state (clone out before any await)
    let registry_key = {
        let communities = state.communities.read();
        let cs = communities.get(community_id)?;
        cs.member_registry_key.clone()?
    };

    // Look up peer's subkey_index from SQLite
    let app_handle = state.app_handle.read().clone();
    let ah = app_handle.as_ref()?;
    let pool = ah.try_state::<crate::db::DbPool>()?;
    let cid = community_id.to_string();
    let pk = peer_pseudonym.to_string();
    let subkey_index = crate::db_helpers::db_call(&pool, move |conn| {
        conn.query_row(
            "SELECT subkey_index FROM community_members WHERE community_id = ?1 AND pseudonym_key = ?2",
            rusqlite::params![cid, pk],
            |row| row.get::<_, u32>(0),
        ).ok().ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)
    }).await.ok()?;

    let rc = state_helpers::routing_context(state)?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    match member_registry::read_member_presence_fresh(&mgr, &registry_key, subkey_index).await {
        Ok(Some(presence)) if presence.status != "offline" => {
            presence.route_blob.filter(|b| !b.is_empty())
        }
        _ => None,
    }
}

/// Update a peer's cached route_blob in the gossip overlay after successful re-resolution.
fn update_peer_route(state: &SharedState, community_id: &str, peer: &str, blob: Vec<u8>) {
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        if let Some(ref mut gossip) = cs.gossip {
            let now = rekindle_utils::timestamp_secs();
            let member = crate::state::OnlineMember {
                route_blob: blob,
                last_seen: now,
            };
            gossip.online_members.insert(peer.to_string(), member.clone());
            if gossip.peers.contains_key(peer) {
                gossip.peers.insert(peer.to_string(), member);
            }
        }
    }
}

/// Extract a dedup key for a locally-originated envelope (before signing).
fn extract_mesh_dedup_key(envelope: &CommunityEnvelope) -> String {
    match envelope {
        CommunityEnvelope::ChatMessage { ref message_id, .. } => message_id.clone(),
        CommunityEnvelope::TypingIndicator {
            ref channel_id,
            ref pseudonym_key,
        } => {
            let bucket = rekindle_utils::timestamp_secs() / 5;
            format!("typing:{channel_id}:{pseudonym_key}:{bucket}")
        }
        CommunityEnvelope::PresenceUpdate {
            ref pseudonym_key, ..
        } => {
            let bucket = rekindle_utils::timestamp_secs() / 30;
            format!("presence:{pseudonym_key}:{bucket}")
        }
        CommunityEnvelope::Control(_) => {
            use blake2::{Blake2b, Digest, digest::consts::U16};
            let bytes = serde_json::to_vec(envelope).unwrap_or_default();
            let mut h = Blake2b::<U16>::new();
            h.update(&bytes);
            hex::encode(h.finalize())
        }
    }
}

/// Leave a community and clean up local state.
///
/// Broadcasts `ControlPayload::MemberLeave` via gossip (which triggers MEK rotation
/// by any admin), then cleans up local state and `SQLite`.
#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    // Broadcast Leave via gossip mesh before cleaning up locally.
    // Best-effort: ignore errors since we're leaving anyway.
    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let _ = send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::MemberLeave {
            pseudonym_key: my_pseudonym_key,
        }),
    );

    // Remove MEK from cache
    state.mek_cache.lock().remove(&community_id);

    // Remove MEK + keypairs from Stronghold
    {
        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            crate::keystore::delete_mek(keystore, &community_id);
            crate::keystore::delete_manifest_keypair(keystore, &community_id);
            crate::keystore::delete_slot_keypair(keystore, &community_id);
            crate::keystore::delete_slot_seed(keystore, &community_id);
            crate::keystore::delete_registry_keypair(keystore, &community_id);
        }
    }

    // Remove from local state
    state.communities.write().remove(&community_id);

    // Remove from SQLite (CASCADE on communities handles channels)
    let owner_key = state_helpers::current_owner_key(state.inner())?;
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

/// Get message history for a community channel.
///
/// Queries local `SQLite` for cached messages. Messages arrive via gossip
/// broadcasts and are stored locally as they come in.
#[tauri::command]
pub async fn get_channel_messages(
    channel_id: String,
    limit: u32,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    let our_key = state_helpers::current_owner_key(state.inner()).unwrap_or_default();

    // Our pseudonym key for this channel's community (for is_own detection)
    let (community_id, my_pseudonym_key) = {
        let communities = state.communities.read();
        let community = communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id));
        match community {
            Some(c) => (
                Some(c.id.clone()),
                c.my_pseudonym_key.clone().unwrap_or_default(),
            ),
            None => (None, String::new()),
        }
    };

    if let Some(ref cid) = community_id {
        require_permission(&state, cid, Permissions::READ_MESSAGE_HISTORY)?;
    }

    // --- Step 1: Query local SQLite (returns immediately) ---
    let channel_id_clone = channel_id.clone();
    let ok = our_key.clone();
    let mpk = my_pseudonym_key.clone();
    let mut messages = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, timestamp FROM messages \
                 WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
                 ORDER BY timestamp DESC LIMIT ?",
        )?;

        let rows = stmt.query_map(rusqlite::params![ok, channel_id_clone, limit], |row| {
            let sender = db::get_str(row, "sender_key");
            // is_own: match against either our owner_key or pseudonym_key
            let is_own = sender == ok || sender == mpk;
            Ok(Message {
                id: db::get_i64(row, "id"),
                sender_id: sender,
                body: db::get_str(row, "body"),
                timestamp: db::get_i64(row, "timestamp"),
                is_own,
                server_message_id: None, // Local DB history — message IDs come via broadcast
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    })
    .await?;

    // Reverse so messages are in chronological order (query was DESC for most-recent)
    messages.reverse();

    tracing::debug!(
        owner_key = %our_key,
        channel_id = %channel_id,
        local_count = messages.len(),
        "loaded channel messages from local DB"
    );

    // In the coordinator model, messages arrive via broadcasts and are stored
    // in local SQLite as they come in. No server fetch path exists.
    let _ = (community_id, app);

    Ok(messages)
}


/// Remove a member from a community.
///
/// The caller must be the community owner or an admin to kick members.
/// Admins cannot kick other admins or the owner.
/// Broadcasts `ControlPayload::Kick` via gossip mesh, which removes the member
/// and triggers MEK rotation by any admin.
#[tauri::command]
pub async fn remove_community_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    require_permission(state.inner(), &community_id, Permissions::KICK_MEMBERS)?;

    // Broadcast Kick via gossip mesh — ephemeral, no DHT persistence needed.
    // Every member receiving the Kick validates sender has KICK_MEMBERS permission.
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::Kick {
            target_pseudonym: pseudonym_key.clone(),
        }),
    )?;

    // Remove from local DB
    let community_id_clone = community_id.clone();
    let pseudonym_key_clone = pseudonym_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![owner_key, community_id_clone, pseudonym_key_clone],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(
        community = %community_id,
        member = %pseudonym_key,
        "removed community member"
    );
    Ok(())
}

/// Get all role definitions for a community from DHT manifest.
#[tauri::command]
pub async fn get_roles(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<CommunityRoleDto>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // Try reading from DHT manifest first
    if let Some(rc) = state_helpers::routing_context(state.inner()) {
        let manifest_key = manifest_key_for(state.inner(), &community_id)?;
        let mgr = rekindle_protocol::dht::DHTManager::new(rc);
        match rekindle_protocol::dht::community::manifest::read_roles(&mgr, &manifest_key).await {
            Ok(entries) => {
                // Cache in memory
                let role_defs: Vec<crate::state::RoleDefinition> = entries
                    .iter()
                    .map(|r| crate::state::RoleDefinition {
                        id: r.id,
                        name: r.name.clone(),
                        color: r.color,
                        permissions: r.permissions,
                        position: r.position,
                        hoist: r.hoist,
                        mentionable: r.mentionable,
                    })
                    .collect();
                {
                    let mut communities = state.communities.write();
                    if let Some(c) = communities.get_mut(&community_id) {
                        c.roles.clone_from(&role_defs);
                        c.my_role =
                            Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
                    }
                }
                // Persist to SQLite
                let cid = community_id.clone();
                let defs = role_defs;
                db_call(pool.inner(), move |conn| {
                    conn.execute(
                        "DELETE FROM community_roles WHERE owner_key = ? AND community_id = ?",
                        rusqlite::params![owner_key, cid],
                    )?;
                    for r in &defs {
                        conn.execute(
                            "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                            rusqlite::params![
                                owner_key, cid, r.id, r.name, r.color,
                                r.permissions.cast_signed(), r.position, i32::from(r.hoist), i32::from(r.mentionable),
                            ],
                        )?;
                    }
                    Ok(())
                })
                .await?;
                return Ok(entries.iter().map(CommunityRoleDto::from).collect());
            }
            Err(e) => {
                tracing::debug!(error = %e, "DHT read_roles failed, falling back to cache");
            }
        }
    }

    // Fallback: return cached roles from memory
    let communities = state.communities.read();
    Ok(communities
        .get(&community_id)
        .map(|c| c.roles.iter().map(CommunityRoleDto::from).collect())
        .unwrap_or_default())
}

/// Create a new role in a community.
///
/// `permissions` is accepted as a string to avoid JavaScript `Number` precision loss
/// on u64 values above `2^53 - 1`.
#[tauri::command]
pub async fn create_role(
    community_id: String,
    name: String,
    color: u32,
    permissions: String,
    hoist: bool,
    mentionable: bool,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<u32, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let permissions_u64: u64 = permissions
        .parse()
        .map_err(|e| format!("invalid permissions: {e}"))?;

    // Generate role ID locally for optimistic state update; coordinator assigns canonical ID via broadcast
    use rand::RngCore;
    let role_id: u32 = rand::rngs::OsRng.next_u32().saturating_add(100); // avoid collision with default roles

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::CreateRole {
            name: name.clone(),
            color,
            permissions: permissions_u64,
            hoist,
            mentionable,
        },
    )
    .await?;

    // Optimistic local state update
    let role_def = crate::state::RoleDefinition {
        id: role_id,
        name: name.clone(),
        color,
        permissions: permissions_u64,
        position: 0, // coordinator will assign real position via broadcast
        hoist,
        mentionable,
    };
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(&community_id) {
            c.roles.push(role_def);
        }
    }
    // Persist to SQLite
    let cid = community_id.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?)",
            rusqlite::params![owner_key, cid, role_id, name, color, permissions_u64.cast_signed(), hoist, mentionable],
        )?;
        Ok(())
    }).await?;
    Ok(role_id)
}

/// Edit an existing role in a community.
///
/// `permissions` is accepted as a string to avoid JavaScript `Number` precision loss
/// on u64 values above `2^53 - 1`.
#[tauri::command]
pub async fn edit_role(
    community_id: String,
    role_id: u32,
    name: Option<String>,
    color: Option<u32>,
    permissions: Option<String>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let permissions_u64: Option<u64> = permissions
        .map(|s| s.parse::<u64>())
        .transpose()
        .map_err(|e| format!("invalid permissions: {e}"))?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::EditRole {
            role_id,
            name: name.clone(),
            color,
            permissions: permissions_u64,
            position,
            hoist,
            mentionable,
        },
    )
    .await?;

    // Optimistic local state update
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(&community_id) {
            if let Some(r) = c.roles.iter_mut().find(|r| r.id == role_id) {
                if let Some(ref n) = name {
                    r.name.clone_from(n);
                }
                if let Some(col) = color {
                    r.color = col;
                }
                if let Some(p) = permissions_u64 {
                    r.permissions = p;
                }
                if let Some(pos) = position {
                    r.position = pos;
                }
                if let Some(h) = hoist {
                    r.hoist = h;
                }
                if let Some(m) = mentionable {
                    r.mentionable = m;
                }
            }
            // Recompute display role in case permissions/name changed
            c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
        }
    }
    // Persist to SQLite
    let cid = community_id.clone();
    db_call(pool.inner(), move |conn| {
        // Build dynamic UPDATE — only set fields that were provided
        let mut sets = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(n) = name { sets.push("name = ?"); params.push(Box::new(n)); }
        if let Some(col) = color { sets.push("color = ?"); params.push(Box::new(col)); }
        if let Some(p) = permissions_u64 { sets.push("permissions = ?"); params.push(Box::new(p.cast_signed())); }
        if let Some(pos) = position { sets.push("position = ?"); params.push(Box::new(pos)); }
        if let Some(h) = hoist { sets.push("hoist = ?"); params.push(Box::new(h)); }
        if let Some(m) = mentionable { sets.push("mentionable = ?"); params.push(Box::new(m)); }
        if !sets.is_empty() {
            let sql = format!(
                "UPDATE community_roles SET {} WHERE owner_key = ? AND community_id = ? AND role_id = ?",
                sets.join(", ")
            );
            params.push(Box::new(owner_key));
            params.push(Box::new(cid));
            params.push(Box::new(role_id));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(std::convert::AsRef::as_ref).collect();
            conn.execute(&sql, param_refs.as_slice())?;
        }
        Ok(())
    }).await?;
    Ok(())
}

/// Delete a role from a community.
#[tauri::command]
pub async fn delete_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::DeleteRole { role_id },
    )
    .await?;

    // Optimistic: remove from in-memory state
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(&community_id) {
            c.roles.retain(|r| r.id != role_id);
            c.my_role_ids.retain(|&id| id != role_id);
            c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
        }
    }
    // Remove from SQLite + scrub from all members' role_ids
    let cid = community_id.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM community_roles WHERE owner_key = ? AND community_id = ? AND role_id = ?",
            rusqlite::params![owner_key, cid, role_id],
        )?;
        // Scrub the deleted role_id from all members' role_ids JSON
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, role_ids FROM community_members WHERE owner_key = ? AND community_id = ?",
        )?;
        let members: Vec<(String, String)> = stmt.query_map(
            rusqlite::params![owner_key, cid],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?
        .filter_map(std::result::Result::ok)
        .collect();
        drop(stmt);
        let rid = role_id;
        for (pk, json) in &members {
            let mut ids: Vec<u32> = serde_json::from_str(json).unwrap_or_default();
            if ids.contains(&rid) {
                ids.retain(|&id| id != rid);
                let new_json = serde_json::to_string(&ids).unwrap_or_default();
                conn.execute(
                    "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![new_json, owner_key, cid, pk],
                )?;
            }
        }
        // Also scrub from the communities.my_role_ids
        let my_ids_json: String = conn.query_row(
            "SELECT my_role_ids FROM communities WHERE owner_key = ? AND id = ?",
            rusqlite::params![owner_key, cid],
            |row| row.get(0),
        ).unwrap_or_else(|_| "[0,1]".to_string());
        let mut my_ids: Vec<u32> = serde_json::from_str(&my_ids_json).unwrap_or_default();
        if my_ids.contains(&rid) {
            my_ids.retain(|&id| id != rid);
            let new_json = serde_json::to_string(&my_ids).unwrap_or_default();
            conn.execute(
                "UPDATE communities SET my_role_ids = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![new_json, owner_key, cid],
            )?;
        }
        Ok(())
    }).await?;
    Ok(())
}

/// Assign a role to a member (additive — does not remove other roles).
#[tauri::command]
pub async fn assign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_ROLES)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::AssignRole {
            target_pseudonym: pseudonym_key.clone(),
            role_id,
        },
    )
    .await?;

    // Optimistic: update in-memory state if target is self
    let is_self = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .is_some_and(|c| c.my_pseudonym_key.as_deref() == Some(&pseudonym_key))
    };
    if is_self {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(&community_id) {
            if !c.my_role_ids.contains(&role_id) {
                c.my_role_ids.push(role_id);
            }
            c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
        }
    }
    // Update SQLite member role_ids
    let cid = community_id.clone();
    let pk = pseudonym_key.clone();
    db_call(pool.inner(), move |conn| {
        let current: String = conn.query_row(
            "SELECT role_ids FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![owner_key, cid, pk],
            |row| row.get(0),
        ).unwrap_or_else(|_| "[0,1]".to_string());
        let mut ids: Vec<u32> = serde_json::from_str(&current).unwrap_or_default();
        if !ids.contains(&role_id) {
            ids.push(role_id);
        }
        let new_json = serde_json::to_string(&ids).unwrap_or_default();
        conn.execute(
            "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![new_json, owner_key, cid, pk],
        )?;
        Ok(())
    }).await?;
    Ok(())
}

/// Remove a role from a member.
#[tauri::command]
pub async fn unassign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_ROLES)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::UnassignRole {
            target_pseudonym: pseudonym_key.clone(),
            role_id,
        },
    )
    .await?;

    // Optimistic: update in-memory state if target is self
    let is_self = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .is_some_and(|c| c.my_pseudonym_key.as_deref() == Some(&pseudonym_key))
    };
    if is_self {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(&community_id) {
            c.my_role_ids.retain(|&id| id != role_id);
            c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
        }
    }
    // Update SQLite member role_ids
    let cid = community_id.clone();
    let pk = pseudonym_key.clone();
    db_call(pool.inner(), move |conn| {
        let current: String = conn.query_row(
            "SELECT role_ids FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![owner_key, cid, pk],
            |row| row.get(0),
        ).unwrap_or_else(|_| "[0,1]".to_string());
        let mut ids: Vec<u32> = serde_json::from_str(&current).unwrap_or_default();
        ids.retain(|&id| id != role_id);
        let new_json = serde_json::to_string(&ids).unwrap_or_default();
        conn.execute(
            "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![new_json, owner_key, cid, pk],
        )?;
        Ok(())
    }).await?;
    Ok(())
}

/// Timeout a member (prevent sending for a duration).
#[tauri::command]
pub async fn timeout_member(
    community_id: String,
    pseudonym_key: String,
    duration_seconds: u64,
    reason: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MODERATE_MEMBERS)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // Broadcast timeout via gossip mesh — ephemeral (expires after duration).
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::TimeoutMember {
            target_pseudonym: pseudonym_key.clone(),
            duration_seconds,
            reason,
        }),
    )?;

    // Optimistic: compute timeout_until and persist to SQLite
    let timeout_until = db::timestamp_now() / 1000 + duration_seconds.cast_signed();
    let cid = community_id.clone();
    let pk = pseudonym_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "UPDATE community_members SET timeout_until = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![timeout_until, owner_key, cid, pk],
        )?;
        Ok(())
    }).await?;
    Ok(())
}

/// Remove a member's timeout.
#[tauri::command]
pub async fn remove_timeout(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MODERATE_MEMBERS)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // Broadcast timeout removal via gossip mesh.
    send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::RemoveTimeout {
            target_pseudonym: pseudonym_key.clone(),
        }),
    )?;

    // Optimistic: clear timeout in SQLite
    let cid = community_id.clone();
    let pk = pseudonym_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "UPDATE community_members SET timeout_until = NULL WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![owner_key, cid, pk],
        )?;
        Ok(())
    }).await?;
    Ok(())
}

/// Set a channel permission overwrite.
#[tauri::command]
pub async fn set_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    allow: u64,
    deny: u64,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::SetChannelOverwrite {
            channel_id: channel_id.clone(),
            target_type: target_type.clone(),
            target_id: target_id.clone(),
            allow,
            deny,
        },
    )
    .await?;

    // Optimistic: persist overwrite to local SQLite
    let comm_id = community_id.clone();
    let chan_id = channel_id.clone();
    let tgt_type = target_type.clone();
    let tgt_id = target_id.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO channel_overwrites (owner_key, community_id, channel_id, target_type, target_id, allow, deny) VALUES (?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![owner_key, comm_id, chan_id, tgt_type, tgt_id, allow.cast_signed(), deny.cast_signed()],
        )?;
        Ok(())
    }).await?;
    Ok(())
}

/// Set slowmode delay for a channel (0 to disable).
#[tauri::command]
pub async fn set_slowmode(
    community_id: String,
    channel_id: String,
    seconds: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool;
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::SetSlowmode {
            channel_id: channel_id.clone(),
            seconds,
        },
    )
    .await?;

    // Optimistic: update local state
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
            ch.slowmode_seconds = Some(seconds);
        }
    }
    Ok(())
}

/// Delete a channel permission overwrite.
#[tauri::command]
pub async fn delete_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::DeleteChannelOverwrite {
            channel_id: channel_id.clone(),
            target_type: target_type.clone(),
            target_id: target_id.clone(),
        },
    )
    .await?;

    // Optimistic: remove overwrite from local SQLite
    let comm_id = community_id.clone();
    let chan_id = channel_id.clone();
    let tgt_type = target_type.clone();
    let tgt_id = target_id.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM channel_overwrites WHERE owner_key = ? AND community_id = ? AND channel_id = ? AND target_type = ? AND target_id = ?",
            rusqlite::params![owner_key, comm_id, chan_id, tgt_type, tgt_id],
        )?;
        Ok(())
    }).await?;
    Ok(())
}

/// Delete a channel from a community.
///
/// Persists `ControlPayload::DeleteChannel` to DHT via `execute_state_op`,
/// then removes the channel from local state and `SQLite`.
#[tauri::command]
pub async fn delete_channel(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::DeleteChannel {
            channel_id: channel_id.clone(),
        },
    )
    .await?;

    // Remove from local state
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            community.channels.retain(|ch| ch.id != channel_id);
        }
    }

    // Remove from local SQLite
    let community_id_clone = community_id.clone();
    let channel_id_clone = channel_id.clone();
    db_call(pool.inner(), move |conn| {
        crate::channel_repo::delete_channel(conn, &owner_key, &channel_id_clone, &community_id_clone)?;
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, channel = %channel_id, "channel deleted");
    Ok(())
}

/// Rename a channel in a community.
#[tauri::command]
pub async fn rename_channel(
    community_id: String,
    channel_id: String,
    new_name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::RenameChannel {
            channel_id: channel_id.clone(),
            new_name: new_name.clone(),
        },
    )
    .await?;

    // Update local state
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            if let Some(ch) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
                ch.name.clone_from(&new_name);
            }
        }
    }

    // Update local SQLite
    let community_id_clone = community_id.clone();
    let channel_id_clone = channel_id.clone();
    let name_clone = new_name.clone();
    db_call(pool.inner(), move |conn| {
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

/// Update community metadata (name, description).
#[tauri::command]
pub async fn update_community_info(
    community_id: String,
    name: Option<String>,
    description: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::UpdateCommunity {
            name: name.clone(),
            description: description.clone(),
        },
    )
    .await?;

    // Update local state
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            if let Some(ref n) = name {
                community.name.clone_from(n);
            }
            if let Some(ref d) = description {
                community.description = Some(d.clone());
            }
        }
    }

    // Update local SQLite
    let cid = community_id.clone();
    db_call(pool.inner(), move |conn| {
        if let Some(ref n) = name {
            conn.execute(
                "UPDATE communities SET name = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![n, owner_key, cid],
            )?;
        }
        if let Some(ref d) = description {
            conn.execute(
                "UPDATE communities SET description = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![d, owner_key, cid],
            )?;
        }
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, "community info updated");
    Ok(())
}

/// Ban a member from a community.
#[tauri::command]
pub async fn ban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::BAN_MEMBERS)?;
    let _ = pool;
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::Ban {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await?;

    tracing::info!(community = %community_id, member = %pseudonym_key, "member banned");
    Ok(())
}

/// Unban a member from a community.
#[tauri::command]
pub async fn unban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::BAN_MEMBERS)?;
    let _ = pool;
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::Unban {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await?;

    tracing::info!(community = %community_id, member = %pseudonym_key, "member unbanned");
    Ok(())
}

/// Banned member info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BannedMemberInfo {
    pub pseudonym_key: String,
    pub display_name: String,
    pub banned_at: u64,
    pub reason: Option<String>,
    pub banned_by: String,
}

/// Get the list of banned members for a community from DHT manifest.
#[tauri::command]
pub async fn get_ban_list(
    community_id: String,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<Vec<BannedMemberInfo>, String> {
    let rc = state_helpers::routing_context(state.inner()).ok_or("not attached")?;
    let manifest_key = manifest_key_for(state.inner(), &community_id)?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let bans = rekindle_protocol::dht::community::manifest::read_bans(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read bans: {e}"))?;

    Ok(bans
        .into_iter()
        .map(|b| BannedMemberInfo {
            display_name: if b.pseudonym_key.len() > 12 {
                format!("{}…", &b.pseudonym_key[..12])
            } else {
                b.pseudonym_key.clone()
            },
            pseudonym_key: b.pseudonym_key,
            banned_at: b.banned_at,
            reason: b.reason,
            banned_by: b.banned_by,
        })
        .collect())
}

/// Force MEK rotation for a community.
///
/// Any admin with `registry_owner_keypair` can rotate the MEK locally:
/// 1. Generate new MEK with next generation
/// 2. Read member index → wrap MEK per-member via ECDH
/// 3. Write MEK vault to registry DHT subkey 1
/// 4. Update local cache + Stronghold
/// 5. Broadcast `MEKRotated` via gossip so peers fetch from DHT
#[tauri::command]
pub async fn rotate_mek(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore: State<'_, crate::keystore::KeystoreHandle>,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::ADMINISTRATOR)?;

    rotate_mek_local(state.inner(), &community_id, &keystore).await?;

    tracing::info!(community = %community_id, "MEK rotated locally");
    Ok(())
}

/// Perform local MEK rotation: generate, wrap per-member, write vault, broadcast.
pub(crate) async fn rotate_mek_local(
    state: &SharedState,
    community_id: &str,
    keystore: &crate::keystore::KeystoreHandle,
) -> Result<(), String> {
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_crypto::group::mek_distribution::wrap_mek;
    use rekindle_protocol::dht::community::member_registry;
    use rekindle_protocol::dht::community::types::{EncryptedMEKCopy, MEKVaultEntry};

    // 1. Determine next generation
    let current_gen = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map_or(0, |c| c.mek_generation)
    };
    let new_gen = current_gen + 1;

    // 2. Generate new MEK
    let mek = MediaEncryptionKey::generate(new_gen);

    // 3. Get our signing key + pseudonym + registry info
    let (my_signing_key, my_pseudonym, registry_key, registry_owner_kp) = {
        let communities = state.communities.read();
        let c = communities
            .get(community_id)
            .ok_or("community not found")?;
        let registry_key = c
            .member_registry_key
            .clone()
            .ok_or("no member registry key")?;
        let registry_kp = c
            .registry_owner_keypair
            .clone()
            .ok_or("no registry_owner_keypair — only admins can rotate MEK")?;
        let my_pseudonym = c
            .my_pseudonym_key
            .clone()
            .ok_or("no pseudonym key")?;
        let secret = state.identity_secret.lock();
        let signing_key = match *secret {
            Some(ref s) => {
                rekindle_crypto::group::pseudonym::derive_community_pseudonym(s, community_id)
            }
            None => return Err("no identity secret".into()),
        };
        (signing_key, my_pseudonym, registry_key, registry_kp)
    };

    // 4. Open registry writable
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    if let Ok(kp) = registry_owner_kp.parse::<veilid_core::KeyPair>() {
        if let Err(e) = mgr.open_record_writable(&registry_key, kp).await {
            tracing::warn!(error = %e, "failed to open registry writable for MEK rotation");
        }
    }

    // 5. Read member index
    let members = member_registry::read_member_index(&mgr, &registry_key)
        .await
        .map_err(|e| format!("read member index: {e}"))?;

    // 6. Wrap MEK per-member
    let mek_wire = mek.to_wire_bytes();
    let mut copies = Vec::with_capacity(members.len());
    for member in &members {
        let Some(pub_bytes): Option<[u8; 32]> = hex::decode(&member.pseudonym_key)
            .ok()
            .and_then(|b| b.try_into().ok())
        else {
            tracing::warn!(
                member = %member.pseudonym_key,
                "skipping MEK wrap — invalid pseudonym key"
            );
            continue;
        };
        match wrap_mek(&my_signing_key, &pub_bytes, &mek_wire) {
            Ok(encrypted) => {
                copies.push(EncryptedMEKCopy {
                    target_pseudonym: member.pseudonym_key.clone(),
                    encrypted_mek: encrypted,
                });
            }
            Err(e) => {
                tracing::warn!(
                    member = %member.pseudonym_key,
                    error = %e,
                    "failed to wrap MEK for member"
                );
            }
        }
    }

    // 7. Write MEK vault to registry subkey 1
    let vault_entry = MEKVaultEntry {
        channel_id: String::new(), // community-wide MEK
        generation: new_gen,
        rotator_pseudonym: my_pseudonym.clone(),
        copies,
    };
    member_registry::write_mek_vault(&mgr, &registry_key, &[vault_entry])
        .await
        .map_err(|e| format!("write MEK vault: {e}"))?;

    // 8. Update local state
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.mek_generation = new_gen;
        }
    }
    state.mek_cache.lock().insert(community_id.to_string(), mek);

    // 9. Persist to Stronghold
    if let Some(ref ks) = *keystore.lock() {
        if let Some(mek) = state.mek_cache.lock().get(community_id) {
            crate::keystore::persist_mek(ks, community_id, mek);
        }
    }

    // 10. Broadcast MEKRotated via gossip
    let envelope = CommunityEnvelope::Control(ControlPayload::MEKRotated {
        new_generation: new_gen,
    });
    let _ = send_to_mesh(state, community_id, &envelope);

    Ok(())
}

/// Send a typing indicator for a channel in a community.
#[tauri::command]
pub async fn send_channel_typing(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool; // no longer needed

    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };

    let envelope = CommunityEnvelope::TypingIndicator {
        channel_id,
        pseudonym_key,
    };
    send_to_mesh(state.inner(), &community_id, &envelope)
}

/// Update our presence status in a community.
#[tauri::command]
pub async fn update_community_presence(
    community_id: String,
    status: String,
    game_name: Option<String>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool; // no longer needed

    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };

    let game_info = game_name.map(|name| {
        rekindle_protocol::dht::community::envelope::PresenceGameInfo {
            game_name: name,
            game_id,
            elapsed_seconds,
            server_address: server_address.clone(),
        }
    });

    let envelope = CommunityEnvelope::PresenceUpdate {
        pseudonym_key,
        status,
        game_info,
        route_blob: crate::state_helpers::our_route_blob(state.inner()),
    };
    send_to_mesh(state.inner(), &community_id, &envelope)
}

/// Get members of a community from the local cache.
///
/// Community membership is tracked locally -- members are discovered
/// via DHT and cached in `SQLite`. The owner is always included as a
/// member when a community is created.
///
/// Live presence status is cross-referenced from the in-memory friends
/// map so that online friends show their real status instead of "offline".
#[tauri::command]
pub async fn get_community_members(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<MemberDto>, String> {
    // Get our own pseudonym key to identify ourselves in the member list
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
    };
    // Get our own status to show for ourselves
    let my_status =
        state_helpers::identity_status(state.inner()).unwrap_or(crate::state::UserStatus::Online);

    // Get cached role definitions for display name computation
    let role_defs = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .map(|c| c.roles.clone())
            .unwrap_or_default()
    };

    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id_clone = community_id.clone();
    let members = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, display_name, role_ids, timeout_until FROM community_members \
                 WHERE owner_key = ? AND community_id = ? ORDER BY display_name",
        )?;

        let rows = stmt.query_map(rusqlite::params![owner_key, community_id_clone], |row| {
            let pseudonym_key = db::get_str(row, "pseudonym_key");

            // Pseudonym keys are per-community and unlinkable to real identity,
            // so we can't cross-reference with the friends list for presence.
            // Show our own real status; other members default to online
            // (presence tracking via server is a future enhancement).
            let status_str = if my_pseudonym.as_deref() == Some(&pseudonym_key) {
                match my_status {
                    crate::state::UserStatus::Online => "online",
                    crate::state::UserStatus::Away => "away",
                    crate::state::UserStatus::Busy => "busy",
                    crate::state::UserStatus::Offline
                    | crate::state::UserStatus::Invisible => "offline",
                }
            } else {
                "online" // default for other members — server presence tracking TODO
            };

            let role_ids_json = db::get_str(row, "role_ids");
            let role_ids: Vec<u32> =
                serde_json::from_str(&role_ids_json).unwrap_or_else(|_| vec![0, 1]);
            let display_role = crate::state::display_role_name(&role_ids, &role_defs);
            let timeout_until: Option<u64> = row
                .get::<_, Option<i64>>("timeout_until")
                .ok()
                .flatten()
                .map(i64::cast_unsigned);

            Ok(MemberDto {
                pseudonym_key,
                display_name: db::get_str(row, "display_name"),
                role_ids,
                display_role,
                status: status_str.to_string(),
                timeout_until,
            })
        })?;

        let mut members = Vec::new();
        for row in rows {
            members.push(row?);
        }
        Ok(members)
    })
    .await?;

    Ok(members)
}

// Legacy server hosting functions removed — coordinator model replaces server process

// ---------------------------------------------------------------------------
// B.7: Older message pagination
// ---------------------------------------------------------------------------

/// Fetch older messages for pagination before `before_timestamp`.
///
/// In the coordinator model there is no request/response fetch path. This queries
/// local SQLite for messages before the given timestamp. DHT pagination for
/// messages beyond the local DB is a future TODO.
#[tauri::command]
pub async fn get_older_channel_messages(
    community_id: String,
    channel_id: String,
    before_timestamp: u64,
    limit: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    let our_key = state_helpers::current_owner_key(state.inner()).unwrap_or_default();
    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };

    let channel_id_clone = channel_id.clone();
    let ok = our_key.clone();
    let mpk = my_pseudonym_key.clone();
    let before_ts = before_timestamp.cast_signed();
    let mut messages = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, timestamp FROM messages \
             WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
             AND timestamp < ? ORDER BY timestamp DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(rusqlite::params![ok, channel_id_clone, before_ts, limit], |row| {
            let sender = db::get_str(row, "sender_key");
            let is_own = sender == ok || sender == mpk;
            Ok(Message {
                id: db::get_i64(row, "id"),
                sender_id: sender,
                body: db::get_str(row, "body"),
                timestamp: db::get_i64(row, "timestamp"),
                is_own,
                server_message_id: None,
            })
        })?;
        let mut msgs = Vec::new();
        for row in rows {
            msgs.push(row?);
        }
        Ok(msgs)
    })
    .await?;

    // Reverse so messages are in chronological order
    messages.reverse();
    // TODO: DHT pagination for messages beyond local DB
    Ok(messages)
}

// ---------------------------------------------------------------------------
// C.3: Channel topics
// ---------------------------------------------------------------------------

/// Set a channel's topic/description.
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
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::SetChannelTopic {
            channel_id: channel_id.clone(),
            topic: topic.clone(),
        },
    )
    .await?;

    // Optimistic local state update
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
            ch.topic = topic;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// C.4: Channel reordering
// ---------------------------------------------------------------------------

/// Reorder channels within a community.
#[tauri::command]
pub async fn reorder_channels(
    community_id: String,
    channel_ids: Vec<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    execute_state_op(
        state.inner(),
        &community_id,
        ControlPayload::ReorderChannels {
            channel_ids: channel_ids.clone(),
        },
    )
    .await?;

    // Optimistic: reorder channels in memory to match the specified order
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        community.channels.sort_by_key(|ch| {
            channel_ids.iter().position(|id| id == &ch.id).unwrap_or(usize::MAX)
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// B.8: Unread tracking
// ---------------------------------------------------------------------------

/// Mark a channel as read up to a specific message.
///
/// Local-only operation — zeroes the in-memory `unread_count`. No need to broadcast
/// read receipts to peers in a P2P community.
#[tauri::command]
pub async fn mark_channel_read(
    community_id: String,
    channel_id: String,
    last_message_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = (pool, last_message_id);

    // Zero out the local unread count for this channel
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
            ch.unread_count = 0;
        }
    }
    Ok(())
}

/// Unread count entry returned to the frontend.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCountEntry {
    pub channel_id: String,
    pub unread_count: u32,
}

/// Get unread counts for all channels in a community from local state.
#[tauri::command]
pub async fn get_unread_counts(
    community_id: String,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<Vec<UnreadCountEntry>, String> {
    let communities = state.communities.read();
    let community = communities
        .get(&community_id)
        .ok_or("community not found")?;
    Ok(community
        .channels
        .iter()
        .map(|ch| UnreadCountEntry {
            channel_id: ch.id.clone(),
            unread_count: ch.unread_count,
        })
        .collect())
}

// ── Onboarding & Welcome Screen ──

/// Get the onboarding config for a community.
#[tauri::command]
pub async fn get_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<serde_json::Value, String> {
    let rc = state_helpers::routing_context(&state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let manifest_key = manifest_key_for(&state, &community_id)?;
    let config = rekindle_protocol::dht::community::manifest::read_onboarding(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read onboarding: {e}"))?
        .unwrap_or_default();
    serde_json::to_value(&config).map_err(|e| format!("serialize: {e}"))
}

/// Set the onboarding config for a community (admin only).
#[tauri::command]
pub async fn set_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
    config: serde_json::Value,
) -> Result<(), String> {
    require_permission(&state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let rc = state_helpers::routing_context(&state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let manifest_key = manifest_key_for(&state, &community_id)?;
    let config: rekindle_protocol::dht::community::onboarding::OnboardingConfig =
        serde_json::from_value(config).map_err(|e| format!("invalid config: {e}"))?;
    rekindle_protocol::dht::community::manifest::write_onboarding(&mgr, &manifest_key, &config)
        .await
        .map_err(|e| format!("write onboarding: {e}"))
}

/// Get the welcome screen for a community.
#[tauri::command]
pub async fn get_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<serde_json::Value, String> {
    let rc = state_helpers::routing_context(&state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let manifest_key = manifest_key_for(&state, &community_id)?;
    let screen = rekindle_protocol::dht::community::manifest::read_welcome(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read welcome: {e}"))?
        .unwrap_or_default();
    serde_json::to_value(&screen).map_err(|e| format!("serialize: {e}"))
}

/// Set the welcome screen for a community (admin only).
#[tauri::command]
pub async fn set_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
    screen: serde_json::Value,
) -> Result<(), String> {
    require_permission(&state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let rc = state_helpers::routing_context(&state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let manifest_key = manifest_key_for(&state, &community_id)?;
    let screen: rekindle_protocol::dht::community::onboarding::WelcomeScreen =
        serde_json::from_value(screen).map_err(|e| format!("invalid screen: {e}"))?;
    rekindle_protocol::dht::community::manifest::write_welcome(&mgr, &manifest_key, &screen)
        .await
        .map_err(|e| format!("write welcome: {e}"))
}

/// Submit onboarding answers for a community.
///
/// Broadcasts via gossip mesh — any admin with MANAGE_MEMBERS processes it.
#[tauri::command]
pub async fn submit_onboarding_answers(
    state: State<'_, SharedState>,
    community_id: String,
    answers: Vec<serde_json::Value>,
) -> Result<(), String> {
    let answers: Vec<rekindle_protocol::dht::community::envelope::OnboardingAnswer> = answers
        .into_iter()
        .map(|v| serde_json::from_value(v).map_err(|e| format!("invalid answer: {e}")))
        .collect::<Result<Vec<_>, _>>()?;

    let envelope = CommunityEnvelope::Control(ControlPayload::SubmitOnboardingAnswers {
        answers,
    });
    send_to_mesh(state.inner(), &community_id, &envelope)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GossipDiagnostics {
    pub community_id: String,
    pub has_gossip: bool,
    pub gossip_peer_count: usize,
    pub online_member_count: usize,
    pub known_member_count: usize,
    pub needs_initial_sync: bool,
    pub lamport_counter: u64,
    pub has_route_blob: bool,
    pub my_pseudonym_key: Option<String>,
    pub my_subkey_index: Option<u32>,
    pub has_slot_keypair: bool,
    pub has_slot_seed: bool,
    pub has_mek: bool,
    pub coordinator_pseudonym: Option<String>,
    pub gossip_peer_keys: Vec<String>,
    pub online_member_keys: Vec<String>,
}

#[tauri::command]
pub async fn debug_gossip_state(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<GossipDiagnostics, String> {
    let communities = state.communities.read();
    let cs = communities.get(&community_id).ok_or("community not found")?;

    let has_route_blob = state_helpers::our_route_blob(state.inner())
        .is_some_and(|b| !b.is_empty());

    let has_mek = state.mek_cache.lock().contains_key(&community_id);

    let (has_gossip, peer_count, online_count, needs_sync, lamport, peer_keys, online_keys) =
        if let Some(ref g) = cs.gossip {
            (
                true,
                g.peers.len(),
                g.online_members.len(),
                g.needs_initial_sync,
                g.lamport_counter,
                g.peers.keys().cloned().collect::<Vec<_>>(),
                g.online_members.keys().cloned().collect::<Vec<_>>(),
            )
        } else {
            (false, 0, 0, true, 0, vec![], vec![])
        };

    Ok(GossipDiagnostics {
        community_id,
        has_gossip,
        gossip_peer_count: peer_count,
        online_member_count: online_count,
        known_member_count: cs.known_members.len(),
        needs_initial_sync: needs_sync,
        lamport_counter: lamport,
        has_route_blob,
        my_pseudonym_key: cs.my_pseudonym_key.clone(),
        my_subkey_index: cs.my_subkey_index,
        has_slot_keypair: cs.slot_keypair.is_some(),
        has_slot_seed: cs.slot_seed.is_some(),
        has_mek,
        coordinator_pseudonym: cs.coordinator_pseudonym.clone(),
        gossip_peer_keys: peer_keys,
        online_member_keys: online_keys,
    })
}

// ── Helpers ──

/// Get the manifest key for a community.
fn manifest_key_for(state: &SharedState, community_id: &str) -> Result<String, String> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|c| c.manifest_key.clone().or_else(|| Some(c.id.clone())))
        .ok_or_else(|| "community not found".to_string())
}

