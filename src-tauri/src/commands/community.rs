use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::channels::ChatEvent;
use crate::commands::chat::Message;
use crate::db::{self, DbPool};
use crate::db_helpers::{db_call, db_fire};
use crate::keystore::KeystoreHandle;
use crate::services;
use crate::state::{ChannelType, SharedState};
use crate::state_helpers;

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
    pub code: String,
    pub signature: String,
}

/// Invite info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfoDto {
    pub code: String,
    pub created_by: String,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub expires_at: Option<u64>,
    pub created_at: u64,
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
        })
        .collect();
    Ok(list)
}

/// Create a new community and store it in `AppState` + `SQLite`.
#[tauri::command]
pub async fn create_community(
    _app: tauri::AppHandle,
    name: String,
    standalone: bool,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<String, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id =
        services::community_service::create_community(state.inner(), &name, standalone).await?;

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
    let pseudonym_key = my_pseudonym_key
        .clone()
        .unwrap_or_else(|| creator_key.clone());
    let roles_to_persist = community.roles.clone();
    let mek_gen = community.mek_generation.cast_signed();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        // Owner gets all default role IDs: @everyone(0), members(1), moderator(2), admin(3), owner(4)
        let owner_role_ids = serde_json::to_string(&[0u32, 1, 2, 3, 4]).unwrap_or_default();
        conn.execute(
            "INSERT INTO communities (owner_key, id, name, my_role, my_role_ids, joined_at, dht_record_key, dht_owner_keypair, my_pseudonym_key, mek_generation) \
             VALUES (?, ?, ?, 'owner', ?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, name_clone, owner_role_ids, now, dht_record_key, dht_owner_keypair, pseudonym_key, mek_gen],
        )?;

        // Insert the creator as the first member (using pseudonym)
        conn.execute(
            "INSERT INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, pseudonym_key, creator_name, owner_role_ids, now],
        )?;

        // Insert default channels
        for channel in &community.channels {
            crate::channel_repo::insert_channel(conn, &ok, channel, &community_id_clone)?;
        }

        // Persist default roles
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

        Ok(())
    })
    .await?;

    // Start coordinator service for this community (creator is first coordinator)
    {
        let handle = crate::services::coordinator::start(
            state.inner().clone(),
            community_id.clone(),
        );
        state.coordinator_services.write().insert(community_id.clone(), handle);
    }

    Ok(community_id)
}

/// Join an existing community by ID, optionally with an invite code.
#[tauri::command]
pub async fn join_community(
    community_id: String,
    invite_code: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let server_members =
        services::community_service::join_community(state.inner(), &community_id, invite_code.as_deref())
            .await?;

    let (name, dht_record_key) = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .map(|c| (c.name.clone(), c.dht_record_key.clone()))
            .unwrap_or_default()
    };

    // Read joiner identity outside db_call (parking_lot guard is !Send)
    let joiner_name = state_helpers::identity_display_name(state.inner());

    // Get pseudonym key, mek_generation, and channels from the community state
    let (my_pseudonym_key, mek_generation, channels) = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .map(|c| {
                (
                    c.my_pseudonym_key.clone(),
                    c.mek_generation,
                    c.channels.clone(),
                )
            })
            .unwrap_or_default()
    };
    let pseudonym_key = my_pseudonym_key.unwrap_or_else(|| owner_key.clone());

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

    // Get role_ids and roles from community state (set by join RPC response)
    let (my_role_ids, roles_to_persist) = {
        let communities = state.communities.read();
        match communities.get(&community_id) {
            Some(c) => (c.my_role_ids.clone(), c.roles.clone()),
            None => (vec![0, 1], Vec::new()),
        }
    };
    let role_ids_json = serde_json::to_string(&my_role_ids).unwrap_or_else(|_| "[0,1]".to_string());

    let now = db::timestamp_now();
    let community_id_clone = community_id.clone();
    let ok = owner_key;
    let ok_for_members = ok.clone();
    let pk = pseudonym_key.clone();
    let mg = mek_generation.cast_signed();
    let rij = role_ids_json.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO communities (owner_key, id, name, my_role, my_role_ids, joined_at, dht_record_key, my_pseudonym_key, mek_generation) \
             VALUES (?, ?, ?, 'member', ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, name, rij, now, dht_record_key, pk, mg],
        )?;

        // Add ourselves to the community_members table (using pseudonym)
        conn.execute(
            "INSERT OR IGNORE INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, pk, joiner_name, rij, now],
        )?;

        // Persist channels to SQLite so they survive re-login
        for channel in &channels {
            crate::channel_repo::upsert_channel(conn, &ok, channel, &community_id_clone)?;
        }

        // Persist roles from server
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

    // Persist all members from the server's join response
    if !server_members.is_empty() {
        let members_for_db = server_members;
        let cid_clone = community_id.clone();
        db_call(pool.inner(), move |conn| {
            for m in &members_for_db {
                let role_ids_json =
                    serde_json::to_string(&m.role_ids).unwrap_or_else(|_| "[0,1]".to_string());
                conn.execute(
                    "INSERT OR IGNORE INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                    rusqlite::params![ok_for_members, cid_clone, m.pseudonym_key, m.display_name, role_ids_json, now],
                )?;
            }
            Ok(())
        })
        .await?;
    }

    Ok(())
}

/// Create a new channel in a community.
///
/// For hosted communities, sends a `CommunityRequest::CreateChannel` to the
/// server. For local-only communities, creates the channel locally + DHT.
#[tauri::command]
pub async fn create_channel(
    community_id: String,
    name: String,
    channel_type: String,
    category_id: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // If this is a hosted community, send CreateChannel RPC to the server.
    // send_community_rpc will on-demand fetch the route blob if missing.
    let has_community = {
        let communities = state.communities.read();
        communities.contains_key(&community_id)
    };

    if has_community {
        let response = send_community_rpc(
            state.inner(),
            pool.inner(),
            &community_id,
            rekindle_protocol::messaging::CommunityRequest::CreateChannel {
                name: name.clone(),
                channel_type: channel_type.clone(),
                category_id: category_id.clone(),
            },
        )
        .await;

        match response {
            Ok(rekindle_protocol::messaging::CommunityResponse::ChannelCreated { channel_id }) => {
                // Server created the channel — add it to local state too
                let ch_type: ChannelType = channel_type.parse().unwrap_or(ChannelType::Text);
                let channel = crate::state::ChannelInfo {
                    id: channel_id.clone(),
                    name: name.clone(),
                    channel_type: ch_type,
                    unread_count: 0,
                    category_id: None,
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

                return Ok(channel_id);
            }
            Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
                return Err(format!("server rejected channel creation: {message}"));
            }
            Ok(other) => {
                return Err(format!(
                    "unexpected server response for CreateChannel: {other:?}"
                ));
            }
            Err(e) => {
                tracing::warn!(
                    community = %community_id, error = %e,
                    "server unreachable for CreateChannel — falling back to local-only"
                );
                // Fall through to local-only creation below
            }
        }
    }

    // Local-only channel creation (no server route, or server was unreachable)
    let channel_id = services::community_service::create_channel(
        state.inner(),
        &community_id,
        &name,
        &channel_type,
    )
    .await?;

    let ch_type: ChannelType = channel_type.parse().unwrap_or(ChannelType::Text);
    let channel = crate::state::ChannelInfo {
        id: channel_id.clone(),
        name: name.clone(),
        channel_type: ch_type,
        unread_count: 0,
        category_id: None,
        topic: String::new(),
        slowmode_seconds: None,
        nsfw: false,
        message_record_key: None,
        mek_generation: 0,
    };
    let community_id_clone = community_id.clone();
    db_call(pool.inner(), move |conn| {
        crate::channel_repo::insert_channel(conn, &owner_key, &channel, &community_id_clone)?;
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::CreateCategory { name: name.clone() },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::CategoryCreated { category_id } => {
            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(&community_id) {
                let sort_order =
                    i32::try_from(community.categories.len()).unwrap_or(i32::MAX);
                community.categories.push(crate::state::CategoryInfo {
                    id: category_id.clone(),
                    name,
                    sort_order,
                });
            }
            Ok(category_id)
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected category creation: {message}"))
        }
        other => Err(format!(
            "unexpected server response for CreateCategory: {other:?}"
        )),
    }
}

/// Delete a channel category.
#[tauri::command]
pub async fn delete_category(
    community_id: String,
    category_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::DeleteCategory {
            category_id: category_id.clone(),
        },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::Ok => {
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
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected category deletion: {message}"))
        }
        other => Err(format!(
            "unexpected server response for DeleteCategory: {other:?}"
        )),
    }
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::RenameCategory {
            category_id: category_id.clone(),
            new_name: new_name.clone(),
        },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::Ok => {
            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(&community_id) {
                if let Some(cat) = community.categories.iter_mut().find(|c| c.id == category_id) {
                    cat.name = new_name;
                }
            }
            Ok(())
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected category rename: {message}"))
        }
        other => Err(format!(
            "unexpected server response for RenameCategory: {other:?}"
        )),
    }
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::MoveChannel {
            channel_id: channel_id.clone(),
            category_id: category_id.clone(),
        },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::Ok => {
            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(&community_id) {
                if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
                    ch.category_id = category_id;
                }
            }
            Ok(())
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected move channel: {message}"))
        }
        other => Err(format!(
            "unexpected server response for MoveChannel: {other:?}"
        )),
    }
}

/// Reorder categories within a community.
#[tauri::command]
pub async fn reorder_categories(
    community_id: String,
    category_ids: Vec<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::ReorderCategories {
            category_ids: category_ids.clone(),
        },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::Ok => {
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
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected reorder categories: {message}"))
        }
        other => Err(format!(
            "unexpected server response for ReorderCategories: {other:?}"
        )),
    }
}

// ---------------------------------------------------------------------------
// Invite management
// ---------------------------------------------------------------------------

/// Create a community invite code.
#[tauri::command]
pub async fn create_community_invite(
    community_id: String,
    max_uses: Option<u32>,
    expires_in_seconds: Option<u64>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<InviteCreatedDto, String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::CreateInvite {
            max_uses,
            expires_in_seconds,
        },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::InviteCreated { code, signature } => {
            Ok(InviteCreatedDto { code, signature })
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("failed to create invite: {message}"))
        }
        other => Err(format!("unexpected response for CreateInvite: {other:?}")),
    }
}

/// Revoke a community invite code.
#[tauri::command]
pub async fn revoke_community_invite(
    community_id: String,
    code: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::RevokeInvite { code },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::Ok => Ok(()),
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("failed to revoke invite: {message}"))
        }
        other => Err(format!("unexpected response for RevokeInvite: {other:?}")),
    }
}

/// List active community invites.
#[tauri::command]
pub async fn list_community_invites(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<InviteInfoDto>, String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::ListInvites,
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::InviteList { invites } => {
            Ok(invites
                .into_iter()
                .map(|i| InviteInfoDto {
                    code: i.code,
                    created_by: i.created_by,
                    max_uses: i.max_uses,
                    uses: i.uses,
                    expires_at: i.expires_at,
                    created_at: i.created_at,
                })
                .collect())
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("failed to list invites: {message}"))
        }
        other => Err(format!("unexpected response for ListInvites: {other:?}")),
    }
}

/// Send a message in a community channel.
///
/// Encrypts the message body with the community's MEK, then sends a
/// `CommunityRequest::SendMessage` to the community server via `app_call`.
/// Falls back to local-only storage if the server is unreachable.
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

    // --- Step 4: Send to coordinator (best-effort — message already persisted) ---
    let message_id = format!("msg_{}", hex::encode(rand_nonce().get(..8).unwrap_or(&[0; 8])));
    let delivery_result = send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::ChatMessage {
            channel_id: channel_id.clone(),
            message_id,
            author_pseudonym: sender_key.clone(),
            ciphertext: ciphertext.clone(),
            mek_generation,
            timestamp: timestamp.cast_unsigned(),
            reply_to_id,
        },
    )
    .await;

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
        server_message_id: None, // Local echo — server ID arrives via broadcast
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

    send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::EditMessage {
                channel_id,
                message_id,
                new_ciphertext,
                mek_generation,
            },
        ),
    )
    .await
}

/// Delete a channel message.
///
/// Sends a `DeleteMessage` RPC to the community server. The server checks
/// that the sender owns the message or has `MANAGE_MESSAGES` permission.
#[tauri::command]
pub async fn delete_channel_message(
    channel_id: String,
    message_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool; // no longer needed for coordinator path
    let community_id = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .map(|c| c.id.clone())
            .ok_or("channel not found in any community")?
    };

    send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::DeleteMessage {
                channel_id,
                message_id,
            },
        ),
    )
    .await
}

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
    let _ = pool; // no longer needed for coordinator path
    send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::AddReaction {
                channel_id,
                message_id,
                emoji,
            },
        ),
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
    let _ = pool; // no longer needed for coordinator path
    send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::RemoveReaction {
                channel_id,
                message_id,
                emoji,
            },
        ),
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
    let _ = pool;
    send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::PinMessage {
                channel_id,
                message_id,
            },
        ),
    )
    .await
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
    let _ = pool;
    send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::UnpinMessage {
                channel_id,
                message_id,
            },
        ),
    )
    .await
}

/// Get pinned messages for a community channel.
#[tauri::command]
pub async fn get_channel_pins(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<PinnedMessageInfoDto>, String> {
    let request = rekindle_protocol::messaging::CommunityRequest::GetPins { channel_id };
    let response = send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::PinnedMessages { pins }) => Ok(pins
            .into_iter()
            .map(|p| PinnedMessageInfoDto {
                message_id: p.message_id,
                channel_id: p.channel_id,
                pinned_by: p.pinned_by,
                pinned_at: p.pinned_at,
            })
            .collect()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
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
    // Check VIEW_AUDIT_LOG permission
    {
        use rekindle_protocol::dht::community::permissions_v2::Permissions;
        let communities = state.communities.read();
        let community = communities.get(&community_id).ok_or("community not found")?;
        let my_perms_bits = community
            .my_role_ids
            .iter()
            .filter_map(|rid| community.roles.iter().find(|r| r.id == *rid))
            .fold(0u64, |acc, r| acc | r.permissions);
        let perms = Permissions::from_bits_truncate(my_perms_bits);
        if !perms.contains(Permissions::VIEW_AUDIT_LOG)
            && !perms.contains(Permissions::ADMINISTRATOR)
        {
            return Err("missing VIEW_AUDIT_LOG permission".into());
        }
    }

    // Get audit record key from coordinator service
    let audit_key = {
        let services = state.coordinator_services.read();
        services.get(&community_id).and_then(|h| {
            let logger = h.relay.audit_logger();
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
    let request = rekindle_protocol::messaging::CommunityRequest::CreateEvent {
        title,
        description,
        start_time,
        end_time,
        channel_id,
        max_attendees,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::EventCreated { event_id }) => {
            Ok(event_id)
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
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
    let request = rekindle_protocol::messaging::CommunityRequest::EditEvent {
        event_id,
        title,
        description,
        start_time,
        end_time,
        channel_id,
        max_attendees,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => Ok(()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Delete a community event.
#[tauri::command]
pub async fn delete_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    let request = rekindle_protocol::messaging::CommunityRequest::DeleteEvent {
        event_id,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => Ok(()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Cancel a community event (sets status to "canceled").
#[tauri::command]
pub async fn cancel_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    let request = rekindle_protocol::messaging::CommunityRequest::CancelEvent {
        event_id,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => Ok(()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
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
    let request = rekindle_protocol::messaging::CommunityRequest::RsvpEvent {
        event_id,
        status,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => Ok(()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Get community events.
#[tauri::command]
pub async fn get_events(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<Vec<EventInfoDto>, String> {
    let request = rekindle_protocol::messaging::CommunityRequest::GetEvents;
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::EventList { events }) => {
            Ok(events
                .into_iter()
                .map(|e| EventInfoDto {
                    id: e.id,
                    title: e.title,
                    description: e.description,
                    creator_pseudonym: e.creator_pseudonym,
                    start_time: e.start_time,
                    end_time: e.end_time,
                    channel_id: e.channel_id,
                    max_attendees: e.max_attendees,
                    created_at: e.created_at,
                    status: e.status,
                    rsvps: e
                        .rsvps
                        .into_iter()
                        .map(|r| EventRsvpInfoDto {
                            pseudonym_key: r.pseudonym_key,
                            status: r.status,
                        })
                        .collect(),
                })
                .collect())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
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
    let request = rekindle_protocol::messaging::CommunityRequest::CreateThread {
        channel_id,
        name,
        starter_message_id,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::ThreadCreated { thread_id }) => {
            Ok(thread_id)
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Get threads for a channel.
#[tauri::command]
pub async fn get_channel_threads(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<ThreadInfoDto>, String> {
    let request = rekindle_protocol::messaging::CommunityRequest::GetChannelThreads {
        channel_id,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::ThreadList { threads }) => {
            Ok(threads
                .into_iter()
                .map(|t| ThreadInfoDto {
                    id: t.id,
                    channel_id: t.channel_id,
                    name: t.name,
                    starter_message_id: t.starter_message_id,
                    creator_pseudonym: t.creator_pseudonym,
                    created_at: t.created_at,
                    archived: t.archived,
                    auto_archive_seconds: t.auto_archive_seconds,
                    last_message_at: t.last_message_at,
                    message_count: t.message_count,
                })
                .collect())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
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

    let request = rekindle_protocol::messaging::CommunityRequest::SendThreadMessage {
        thread_id,
        ciphertext,
        mek_generation,
        reply_to_id: None,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(
            rekindle_protocol::messaging::CommunityResponse::Ok
            | rekindle_protocol::messaging::CommunityResponse::MessageSent { .. },
        ) => Ok(()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Get thread message history (decrypted with MEK).
#[tauri::command]
pub async fn get_thread_messages(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
    limit: u32,
    before_timestamp: Option<u64>,
) -> Result<Vec<Message>, String> {
    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };

    let request = rekindle_protocol::messaging::CommunityRequest::GetThreadMessages {
        thread_id,
        limit,
        before_timestamp,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::ThreadMessages { messages }) => {
            // Decrypt with cached MEK
            let mek_cache = state.mek_cache.lock();
            let Some(mek) = mek_cache.get(&community_id) else {
                return Err("no MEK to decrypt thread messages".into());
            };

            let mut result = Vec::new();
            for msg in &messages {
                if msg.mek_generation != mek.generation() {
                    continue;
                }
                match mek.decrypt(&msg.ciphertext) {
                    Ok(plaintext) => {
                        let body = String::from_utf8(plaintext).unwrap_or_default();
                        let is_own = msg.sender_pseudonym == my_pseudonym_key;
                        result.push(Message {
                            id: 0,
                            sender_id: msg.sender_pseudonym.clone(),
                            body,
                            timestamp: msg.timestamp.cast_signed(),
                            is_own,
                            server_message_id: Some(msg.message_id.clone()),
                        });
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "failed to decrypt thread message");
                    }
                }
            }
            Ok(result)
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Archive a thread.
#[tauri::command]
pub async fn archive_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
) -> Result<(), String> {
    let request = rekindle_protocol::messaging::CommunityRequest::ArchiveThread {
        thread_id,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => Ok(()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Unarchive a thread.
#[tauri::command]
pub async fn unarchive_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
) -> Result<(), String> {
    let request = rekindle_protocol::messaging::CommunityRequest::UnarchiveThread {
        thread_id,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => Ok(()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
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
    let request = rekindle_protocol::messaging::CommunityRequest::AddGameServer {
        game_id,
        label,
        address,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::GameServerList { servers }) => {
            Ok(servers.first().map_or_else(String::new, |s| s.id.clone()))
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Remove a game server from a community's favorites.
#[tauri::command]
pub async fn remove_game_server(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    server_id: String,
) -> Result<(), String> {
    let request = rekindle_protocol::messaging::CommunityRequest::RemoveGameServer { server_id };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => Ok(()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
}

/// Get all game servers for a community.
#[tauri::command]
pub async fn get_game_servers(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<Vec<GameServerInfoDto>, String> {
    let request = rekindle_protocol::messaging::CommunityRequest::GetGameServers;
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await;
    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::GameServerList { servers }) => {
            Ok(servers
                .into_iter()
                .map(|s| GameServerInfoDto {
                    id: s.id,
                    game_id: s.game_id,
                    label: s.label,
                    address: s.address,
                    added_by: s.added_by,
                    created_at: s.created_at,
                })
                .collect())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(message),
        Ok(_) => Err("unexpected response".into()),
        Err(e) => Err(e),
    }
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

fn rand_nonce() -> Vec<u8> {
    use rand::RngCore;
    let mut nonce = vec![0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Send a community RPC request to the server.
///
/// Send a CommunityEnvelope to the coordinator via `app_message` (fire-and-forget).
///
/// Replaces `send_community_rpc()` — no request/response cycle. The coordinator
/// validates, relays, and persists; members receive the result via broadcast.
pub(crate) async fn send_to_coordinator(
    state: &SharedState,
    community_id: &str,
    envelope: rekindle_protocol::dht::community::envelope::CommunityEnvelope,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope;

    // Get coordinator route and our pseudonym key
    let (coordinator_route_blob, my_pseudonym_key) = {
        let communities = state.communities.read();
        let c = communities
            .get(community_id)
            .ok_or("community not found")?;
        (
            c.coordinator_route_blob.clone(),
            c.my_pseudonym_key
                .clone()
                .unwrap_or_default(),
        )
    };

    let route_blob = coordinator_route_blob
        .ok_or("no coordinator available — message will be queued")?;

    // Sign envelope with pseudonym signing key
    let signing_key = {
        let secret = state.identity_secret.lock();
        let s = (*secret).ok_or("identity not unlocked")?;
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&s, community_id)
    };
    let envelope_bytes =
        serde_json::to_vec(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
    let signed = envelope::sign_envelope(
        &signing_key,
        community_id,
        &my_pseudonym_key,
        &envelope_bytes,
    );
    let signed_bytes =
        serde_json::to_vec(&signed).map_err(|e| format!("serialize signed: {e}"))?;

    // Send via app_message (fire-and-forget, not app_call)
    let rc = state_helpers::routing_context(state).ok_or("Veilid network not attached")?;
    let route_id =
        state_helpers::import_route_blob(state, &route_blob).map_err(|e| format!("route: {e}"))?;
    rc.app_message(veilid_core::Target::RouteId(route_id), signed_bytes)
        .await
        .map_err(|e| format!("app_message: {e}"))
}

/// Legacy compatibility wrapper: converts `CommunityRequest` to `ControlPayload`
/// and routes through the coordinator. Write operations return `CommunityResponse::Ok`;
/// read operations that need server-side data will fail and should be migrated
/// to read from DHT directly.
pub(crate) async fn send_community_rpc(
    state: &SharedState,
    _pool: &DbPool,
    community_id: &str,
    request: rekindle_protocol::messaging::CommunityRequest,
) -> Result<rekindle_protocol::messaging::CommunityResponse, String> {
    use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

    let control = request_to_control_payload(request)?;
    send_to_coordinator(state, community_id, CommunityEnvelope::Control(control)).await?;
    Ok(rekindle_protocol::messaging::CommunityResponse::Ok)
}

/// Convert a legacy CommunityRequest to the v2 ControlPayload.
fn request_to_control_payload(
    request: rekindle_protocol::messaging::CommunityRequest,
) -> Result<rekindle_protocol::dht::community::envelope::ControlPayload, String> {
    use rekindle_protocol::dht::community::envelope::ControlPayload;
    use rekindle_protocol::messaging::CommunityRequest;

    Ok(match request {
        // Channel management
        CommunityRequest::CreateChannel { name, channel_type, category_id } =>
            ControlPayload::CreateChannel { name, channel_type, category_id },
        CommunityRequest::DeleteChannel { channel_id } =>
            ControlPayload::DeleteChannel { channel_id },
        CommunityRequest::RenameChannel { channel_id, new_name } =>
            ControlPayload::RenameChannel { channel_id, new_name },
        CommunityRequest::SetChannelTopic { channel_id, topic } =>
            ControlPayload::SetChannelTopic { channel_id, topic },
        CommunityRequest::ReorderChannels { channel_ids } =>
            ControlPayload::ReorderChannels { channel_ids },
        CommunityRequest::SetSlowmode { channel_id, seconds } =>
            ControlPayload::SetSlowmode { channel_id, seconds },
        CommunityRequest::MoveChannel { channel_id, category_id } =>
            ControlPayload::MoveChannel { channel_id, category_id },

        // Category management
        CommunityRequest::CreateCategory { name } =>
            ControlPayload::CreateCategory { name },
        CommunityRequest::DeleteCategory { category_id } =>
            ControlPayload::DeleteCategory { category_id },
        CommunityRequest::RenameCategory { category_id, new_name } =>
            ControlPayload::RenameCategory { category_id, new_name },
        CommunityRequest::ReorderCategories { category_ids } =>
            ControlPayload::ReorderCategories { category_ids },

        // Messages
        CommunityRequest::SendMessage { channel_id, ciphertext, mek_generation, reply_to_id: _ } =>
            ControlPayload::EditMessage { channel_id, message_id: String::new(), new_ciphertext: ciphertext, mek_generation },
        CommunityRequest::EditMessage { channel_id, message_id, new_ciphertext, mek_generation } =>
            ControlPayload::EditMessage { channel_id, message_id, new_ciphertext, mek_generation },
        CommunityRequest::DeleteMessage { channel_id, message_id } =>
            ControlPayload::DeleteMessage { channel_id, message_id },

        // Moderation
        CommunityRequest::Kick { target_pseudonym } =>
            ControlPayload::Kick { target_pseudonym },
        CommunityRequest::Ban { target_pseudonym } =>
            ControlPayload::Ban { target_pseudonym },
        CommunityRequest::Unban { target_pseudonym } =>
            ControlPayload::Unban { target_pseudonym },
        CommunityRequest::TimeoutMember { target_pseudonym, duration_seconds, reason } =>
            ControlPayload::TimeoutMember { target_pseudonym, duration_seconds, reason },
        CommunityRequest::RemoveTimeout { target_pseudonym } =>
            ControlPayload::RemoveTimeout { target_pseudonym },

        // Roles
        CommunityRequest::CreateRole { name, color, permissions, hoist, mentionable } =>
            ControlPayload::CreateRole { name, color, permissions, hoist, mentionable },
        CommunityRequest::EditRole { role_id, name, color, permissions, position, hoist, mentionable } =>
            ControlPayload::EditRole { role_id, name, color, permissions, position, hoist, mentionable },
        CommunityRequest::DeleteRole { role_id } =>
            ControlPayload::DeleteRole { role_id },
        CommunityRequest::AssignRole { target_pseudonym, role_id } =>
            ControlPayload::AssignRole { target_pseudonym, role_id },
        CommunityRequest::UnassignRole { target_pseudonym, role_id } =>
            ControlPayload::UnassignRole { target_pseudonym, role_id },

        // Invites
        CommunityRequest::CreateInvite { max_uses, expires_in_seconds } =>
            ControlPayload::CreateInvite { max_uses, expires_in_seconds },
        CommunityRequest::RevokeInvite { code } =>
            ControlPayload::RevokeInvite { code },

        // Events
        CommunityRequest::CreateEvent { title, description, start_time, end_time, channel_id, max_attendees } =>
            ControlPayload::CreateEvent { title, description, start_time, end_time, channel_id, max_attendees },
        CommunityRequest::EditEvent { event_id, title, description, start_time, end_time, channel_id, max_attendees } =>
            ControlPayload::EditEvent { event_id, title, description, start_time, end_time, channel_id, max_attendees },
        CommunityRequest::DeleteEvent { event_id } =>
            ControlPayload::DeleteEvent { event_id },
        CommunityRequest::CancelEvent { event_id } =>
            ControlPayload::CancelEvent { event_id },
        CommunityRequest::RsvpEvent { event_id, status } =>
            ControlPayload::RsvpEvent { event_id, status },

        // Reactions
        CommunityRequest::AddReaction { channel_id, message_id, emoji } =>
            ControlPayload::AddReaction { channel_id, message_id, emoji },
        CommunityRequest::RemoveReaction { channel_id, message_id, emoji } =>
            ControlPayload::RemoveReaction { channel_id, message_id, emoji },

        // Pins
        CommunityRequest::PinMessage { channel_id, message_id } =>
            ControlPayload::PinMessage { channel_id, message_id },
        CommunityRequest::UnpinMessage { channel_id, message_id } =>
            ControlPayload::UnpinMessage { channel_id, message_id },

        // Threads
        CommunityRequest::CreateThread { channel_id, name, starter_message_id } =>
            ControlPayload::CreateThread { channel_id, name, starter_message_id },
        CommunityRequest::SendThreadMessage { thread_id, ciphertext, mek_generation, reply_to_id } =>
            ControlPayload::SendThreadMessage { thread_id, ciphertext, mek_generation, reply_to_id },
        CommunityRequest::ArchiveThread { thread_id } =>
            ControlPayload::ArchiveThread { thread_id },
        CommunityRequest::UnarchiveThread { thread_id } =>
            ControlPayload::UnarchiveThread { thread_id },

        // Game servers
        CommunityRequest::AddGameServer { game_id, label, address } =>
            ControlPayload::AddGameServer { game_id, label, address },
        CommunityRequest::RemoveGameServer { server_id } =>
            ControlPayload::RemoveGameServer { server_id },

        // Channel overwrites
        CommunityRequest::SetChannelOverwrite { channel_id, target_type, target_id, allow, deny } =>
            ControlPayload::SetChannelOverwrite { channel_id, target_type, target_id, allow, deny },
        CommunityRequest::DeleteChannelOverwrite { channel_id, target_type, target_id } =>
            ControlPayload::DeleteChannelOverwrite { channel_id, target_type, target_id },

        // Community metadata
        CommunityRequest::UpdateCommunity { name, description } =>
            ControlPayload::UpdateCommunity { name, description },

        // Presence — ChannelTyping handled directly by send_channel_typing, but map for legacy callers
        CommunityRequest::ChannelTyping { channel_id: _ } =>
            ControlPayload::UpdatePresence { status: "typing".into(), game_name: None, game_id: None, elapsed_seconds: None, server_address: None },
        CommunityRequest::UpdatePresence { status, game_name, game_id, elapsed_seconds, server_address } =>
            ControlPayload::UpdatePresence { status, game_name, game_id, elapsed_seconds, server_address },

        // Leave
        CommunityRequest::Leave =>
            ControlPayload::MemberLeave { pseudonym_key: String::new() },

        // MEK
        CommunityRequest::RotateMEK =>
            ControlPayload::RotateMEK,
        CommunityRequest::RequestMEK =>
            ControlPayload::RequestMEK,

        // Read-only mark operations
        CommunityRequest::MarkChannelRead { channel_id, last_message_id } =>
            ControlPayload::MarkChannelRead { channel_id, last_message_id },

        // Read operations — should query DHT directly
        CommunityRequest::GetMessages { .. }
        | CommunityRequest::GetRoles
        | CommunityRequest::GetPins { .. }
        | CommunityRequest::GetBanList
        | CommunityRequest::ListInvites
        | CommunityRequest::GetEvents
        | CommunityRequest::GetChannelThreads { .. }
        | CommunityRequest::GetThreadMessages { .. }
        | CommunityRequest::GetGameServers
        | CommunityRequest::GetUnreadCounts
        | CommunityRequest::GetAuditLog { .. } => {
            return Err("read operations should query DHT directly".into());
        }

        // Join is handled by community_service, not through this path
        CommunityRequest::Join { .. } => {
            return Err("join should go through community_service".into());
        }
    })
}

// Legacy IPC and Veilid RPC functions removed — all traffic routes through coordinator

/// Leave a community and clean up local state.
///
/// Sends `CommunityRequest::Leave` to the server (which triggers MEK rotation
/// for remaining members), then cleans up local state and `SQLite`.
#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    // Send Leave RPC to the community server before cleaning up locally
    // Best-effort: ignore errors since we're leaving anyway
    let _ = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::Leave,
    )
    .await;

    // Remove MEK from cache
    state.mek_cache.lock().remove(&community_id);

    // Remove MEK from Stronghold
    {
        use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
        use rekindle_crypto::Keychain as _;

        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            let key_name = mek_key_name(&community_id);
            if let Err(e) = keystore.delete_key(VAULT_COMMUNITIES, &key_name) {
                tracing::warn!(error = %e, "failed to remove MEK from Stronghold");
            }
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
/// First queries local `SQLite`. If local DB has no messages for the channel,
/// fetches history from the community server via `CommunityRequest::GetMessages`,
/// decrypts the ciphertexts with the cached MEK, and stores them locally.
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
                server_message_id: None, // Local DB history — server IDs come via ChannelHistoryLoaded
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

    // --- Step 2: Background server fetch for missed messages ---
    // Spawn a background task so the frontend gets local messages immediately.
    if let Some(cid) = community_id {
        let state = state.inner().clone();
        let pool = pool.inner().clone();
        let channel_id = channel_id.clone();
        let our_key = our_key.clone();
        let my_pseudonym_key = my_pseudonym_key.clone();
        tokio::spawn(async move {
            let server_messages = fetch_channel_history_from_server(
                &state,
                &pool,
                &cid,
                &channel_id,
                &our_key,
                &my_pseudonym_key,
                limit,
            )
            .await;
            if !server_messages.is_empty() {
                tracing::debug!(
                    channel_id = %channel_id,
                    server_count = server_messages.len(),
                    "background server fetch returned messages"
                );
                let _ = app.emit(
                    "chat-event",
                    ChatEvent::ChannelHistoryLoaded {
                        channel_id,
                        messages: server_messages,
                    },
                );
            }
        });
    }

    Ok(messages)
}

/// Fetch message history from the community server, decrypt, and store locally.
async fn fetch_channel_history_from_server(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    owner_key: &str,
    my_pseudonym_key: &str,
    limit: u32,
) -> Vec<Message> {
    let response = send_community_rpc(
        state,
        pool,
        community_id,
        rekindle_protocol::messaging::CommunityRequest::GetMessages {
            channel_id: channel_id.to_string(),
            before_timestamp: None,
            limit,
        },
    )
    .await;

    let Ok(rekindle_protocol::messaging::CommunityResponse::Messages {
        messages: server_messages,
    }) = response
    else {
        return Vec::new();
    };

    if server_messages.is_empty() {
        return Vec::new();
    }

    // Decrypt with cached MEK — scope the guard so it's dropped before any .await
    let decrypted: Vec<(String, String, String, i64, i64)> = {
        let mek_cache = state.mek_cache.lock();
        let Some(mek) = mek_cache.get(community_id) else {
            tracing::warn!(community = %community_id, "no MEK to decrypt server history");
            return Vec::new();
        };

        let mut result = Vec::new();
        for msg in &server_messages {
            if msg.mek_generation != mek.generation() {
                tracing::debug!(
                    have = mek.generation(),
                    need = msg.mek_generation,
                    "skipping message with different MEK generation"
                );
                continue;
            }
            match mek.decrypt(&msg.ciphertext) {
                Ok(plaintext) => {
                    let body = String::from_utf8(plaintext).unwrap_or_default();
                    result.push((
                        msg.sender_pseudonym.clone(),
                        msg.message_id.clone(),
                        body,
                        msg.timestamp.cast_signed(),
                        msg.mek_generation.cast_signed(),
                    ));
                }
                Err(e) => {
                    tracing::debug!(error = %e, "failed to decrypt historical message");
                }
            }
        }
        result
    };

    // Store decrypted messages in local SQLite (fire-and-forget)
    let ok = owner_key.to_string();
    let cid = channel_id.to_string();
    let mpk = my_pseudonym_key.to_string();
    let decrypted_clone = decrypted.clone();
    db_fire(pool, "store decrypted channel history", move |conn| {
        for (sender, _message_id, body, ts, mg) in &decrypted_clone {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read, mek_generation) \
                 VALUES (?, ?, 'channel', ?, ?, ?, 0, ?)",
                rusqlite::params![ok, cid, sender, body, ts, mg],
            );
        }
        Ok(())
    });

    // Build Message structs for the frontend
    decrypted
        .into_iter()
        .map(|(sender, message_id, body, ts, _mg)| {
            let is_own = sender == mpk;
            Message {
                id: 0, // temporary — will get real IDs on next query from SQLite
                sender_id: sender,
                body,
                timestamp: ts,
                is_own,
                server_message_id: Some(message_id),
            }
        })
        .collect()
}

/// Remove a member from a community.
///
/// The caller must be the community owner or an admin to kick members.
/// Admins cannot kick other admins or the owner.
/// Sends `CommunityRequest::Kick` to the server, which removes the member
/// and rotates the MEK.
#[tauri::command]
pub async fn remove_community_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // Check caller's role — use display role for backward-compat permission check
    let my_role = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_role.clone())
            .unwrap_or_default()
    };

    if my_role != "owner" && my_role != "admin" {
        return Err(
            "insufficient permissions: must be owner or admin to remove members".to_string(),
        );
    }

    // Send Kick RPC to the community server (local validation passed)
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::Kick {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await?;

    // Check if server rejected the kick
    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected kick: {message}"));
    }

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

/// Get all role definitions for a community from the server.
#[tauri::command]
pub async fn get_roles(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<CommunityRoleDto>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::GetRoles,
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::RolesList { roles }) => {
            // Cache the roles locally in memory
            let role_defs: Vec<crate::state::RoleDefinition> = roles
                .iter()
                .map(crate::state::RoleDefinition::from_dto)
                .collect();
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(&community_id) {
                    c.roles.clone_from(&role_defs);
                    c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
                }
            }
            // Persist to SQLite (DELETE + INSERT)
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
            }).await?;
            Ok(roles.iter().map(CommunityRoleDto::from).collect())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected get_roles: {message}"))
        }
        Err(_) | Ok(_) => {
            // Return cached roles if server is unreachable
            let communities = state.communities.read();
            Ok(communities
                .get(&community_id)
                .map(|c| c.roles.iter().map(CommunityRoleDto::from).collect())
                .unwrap_or_default())
        }
    }
}

/// Create a new role in a community.
#[tauri::command]
pub async fn create_role(
    community_id: String,
    name: String,
    color: u32,
    permissions: u64,
    hoist: bool,
    mentionable: bool,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<u32, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::CreateRole {
            name: name.clone(),
            color,
            permissions,
            hoist,
            mentionable,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::RoleCreated { role_id }) => {
            // Optimistic local state update
            let role_def = crate::state::RoleDefinition {
                id: role_id,
                name: name.clone(),
                color,
                permissions,
                position: 0, // server will assign real position via broadcast
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
                    rusqlite::params![owner_key, cid, role_id, name, color, permissions.cast_signed(), hoist, mentionable],
                )?;
                Ok(())
            }).await?;
            Ok(role_id)
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected create_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Edit an existing role in a community.
#[tauri::command]
pub async fn edit_role(
    community_id: String,
    role_id: u32,
    name: Option<String>,
    color: Option<u32>,
    permissions: Option<u64>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::EditRole {
            role_id,
            name: name.clone(),
            color,
            permissions,
            position,
            hoist,
            mentionable,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
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
                        if let Some(p) = permissions {
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
                if let Some(p) = permissions { sets.push("permissions = ?"); params.push(Box::new(p.cast_signed())); }
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
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected edit_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::DeleteRole { role_id },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Remove from in-memory state
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
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected delete_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
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
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::AssignRole {
            target_pseudonym: pseudonym_key.clone(),
            role_id,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Update in-memory state if target is self
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
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected assign_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
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
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::UnassignRole {
            target_pseudonym: pseudonym_key.clone(),
            role_id,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Update in-memory state if target is self
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
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected unassign_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
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
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::TimeoutMember {
            target_pseudonym: pseudonym_key.clone(),
            duration_seconds,
            reason,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Compute timeout_until and persist to SQLite
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
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected timeout_member: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Remove a member's timeout.
#[tauri::command]
pub async fn remove_timeout(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::RemoveTimeout {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Clear timeout in SQLite
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
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected remove_timeout: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::SetChannelOverwrite {
            channel_id: channel_id.clone(),
            target_type: target_type.clone(),
            target_id: target_id.clone(),
            allow,
            deny,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Persist overwrite to local SQLite
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
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected set_channel_overwrite: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::SetSlowmode {
            channel_id: channel_id.clone(),
            seconds,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Update local store
            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(&community_id) {
                if let Some(ch) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
                    ch.slowmode_seconds = Some(seconds);
                }
            }
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected set_slowmode: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::DeleteChannelOverwrite {
            channel_id: channel_id.clone(),
            target_type: target_type.clone(),
            target_id: target_id.clone(),
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Remove overwrite from local SQLite
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
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(format!(
            "server rejected delete_channel_overwrite: {message}"
        )),
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Delete a channel from a community.
///
/// Sends `CommunityRequest::DeleteChannel` to the server, then removes
/// the channel from local state and `SQLite`.
#[tauri::command]
pub async fn delete_channel(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::DeleteChannel {
            channel_id: channel_id.clone(),
        },
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected channel deletion: {message}"));
    }

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
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::RenameChannel {
            channel_id: channel_id.clone(),
            new_name: new_name.clone(),
        },
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected channel rename: {message}"));
    }

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

    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::UpdateCommunity {
            name: name.clone(),
            description: description.clone(),
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::CommunityUpdated) => {}
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            return Err(format!("server rejected community update: {message}"));
        }
        Ok(_) => {
            return Err("unexpected response from server".into());
        }
        Err(e) => {
            return Err(e);
        }
    }

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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::Ban {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected ban: {message}"));
    }

    // Remove from local member list (server already kicked them)
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            // Members are stored in community.members in the SolidJS store,
            // but on the Rust side this is in the DB — the frontend will
            // update its store via the handler.
            let _ = community;
        }
    }

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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::Unban {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected unban: {message}"));
    }

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
}

/// Get the list of banned members for a community.
#[tauri::command]
pub async fn get_ban_list(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<BannedMemberInfo>, String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::GetBanList,
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::BanList { banned }) => Ok(banned
            .into_iter()
            .map(|b| BannedMemberInfo {
                pseudonym_key: b.pseudonym_key,
                display_name: b.display_name,
                banned_at: b.banned_at,
            })
            .collect()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected ban list request: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Force MEK rotation for a community.
#[tauri::command]
pub async fn rotate_mek(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::RotateMEK,
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected MEK rotation: {message}"));
    }

    tracing::info!(community = %community_id, "MEK rotation requested");
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
    let _ = pool; // no longer needed for coordinator path

    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };

    send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        },
    )
    .await
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
    let _ = pool; // no longer needed for coordinator path

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

    send_to_coordinator(
        state.inner(),
        &community_id,
        rekindle_protocol::dht::community::envelope::CommunityEnvelope::PresenceUpdate {
            pseudonym_key,
            status,
            game_info,
            route_blob: crate::state_helpers::our_route_blob(state.inner()),
        },
    )
    .await
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

/// Fetch older messages directly from the community server (no local DB).
///
/// Used for loading message history before the oldest loaded message.
/// Returns decrypted messages older than `before_timestamp`.
#[tauri::command]
pub async fn get_older_channel_messages(
    community_id: String,
    channel_id: String,
    before_timestamp: u64,
    limit: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let our_key = state_helpers::current_owner_key(state.inner()).unwrap_or_default();

    let request = rekindle_protocol::messaging::CommunityRequest::GetMessages {
        channel_id: channel_id.clone(),
        before_timestamp: Some(before_timestamp),
        limit,
    };
    let response =
        send_community_rpc(state.inner(), pool.inner(), &community_id, request).await?;
    match response {
        rekindle_protocol::messaging::CommunityResponse::Messages { messages } => {
            // Decrypt with cached MEK
            let mek_cache = state.mek_cache.lock();
            let Some(mek) = mek_cache.get(&community_id) else {
                return Err("no MEK to decrypt server messages".into());
            };

            let mut result = Vec::new();
            for msg in &messages {
                if msg.mek_generation != mek.generation() {
                    tracing::debug!(
                        have = mek.generation(),
                        need = msg.mek_generation,
                        "skipping message with different MEK generation"
                    );
                    continue;
                }
                let body = match mek.decrypt(&msg.ciphertext) {
                    Ok(plaintext) => String::from_utf8(plaintext).unwrap_or_default(),
                    Err(e) => {
                        tracing::debug!(error = %e, "failed to decrypt older message");
                        continue;
                    }
                };
                let is_own = msg.sender_pseudonym == my_pseudonym_key
                    || msg.sender_pseudonym == our_key;
                result.push(Message {
                    id: 0, // Server-sourced, no local DB id
                    sender_id: msg.sender_pseudonym.clone(),
                    body,
                    timestamp: msg.timestamp.cast_signed(),
                    is_own,
                    server_message_id: Some(msg.message_id.clone()),
                });
            }
            Ok(result)
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => Err(message),
        _ => Err("unexpected response".into()),
    }
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::SetChannelTopic {
            channel_id: channel_id.clone(),
            topic: topic.clone(),
        },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::Ok => {
            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(&community_id) {
                if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
                    ch.topic = topic;
                }
            }
            Ok(())
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected set channel topic: {message}"))
        }
        other => Err(format!(
            "unexpected server response for SetChannelTopic: {other:?}"
        )),
    }
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
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::ReorderChannels {
            channel_ids: channel_ids.clone(),
        },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::Ok => {
            // Reorder channels in memory to match the specified order
            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(&community_id) {
                community.channels.sort_by_key(|ch| {
                    channel_ids.iter().position(|id| id == &ch.id).unwrap_or(usize::MAX)
                });
            }
            Ok(())
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected reorder channels: {message}"))
        }
        other => Err(format!(
            "unexpected server response for ReorderChannels: {other:?}"
        )),
    }
}

// ---------------------------------------------------------------------------
// B.8: Unread tracking
// ---------------------------------------------------------------------------

/// Mark a channel as read up to a specific message.
///
/// Sends `MarkChannelRead` to the server and zeroes the local `unread_count`.
#[tauri::command]
pub async fn mark_channel_read(
    community_id: String,
    channel_id: String,
    last_message_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::MarkChannelRead {
            channel_id: channel_id.clone(),
            last_message_id,
        },
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::Ok => {
            // Zero out the local unread count for this channel
            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(&community_id) {
                if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
                    ch.unread_count = 0;
                }
            }
            Ok(())
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected mark read: {message}"))
        }
        other => Err(format!(
            "unexpected server response for MarkChannelRead: {other:?}"
        )),
    }
}

/// Unread count entry returned to the frontend.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCountEntry {
    pub channel_id: String,
    pub unread_count: u32,
}

/// Get unread counts for all channels in a community.
#[tauri::command]
pub async fn get_unread_counts(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<UnreadCountEntry>, String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::GetUnreadCounts,
    )
    .await?;

    match response {
        rekindle_protocol::messaging::CommunityResponse::UnreadCounts { counts } => {
            // Also update the in-memory channel unread counts
            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(&community_id) {
                for count in &counts {
                    if let Some(ch) = community
                        .channels
                        .iter_mut()
                        .find(|c| c.id == count.channel_id)
                    {
                        ch.unread_count = count.unread_count;
                    }
                }
            }

            Ok(counts
                .into_iter()
                .map(|c| UnreadCountEntry {
                    channel_id: c.channel_id,
                    unread_count: c.unread_count,
                })
                .collect())
        }
        rekindle_protocol::messaging::CommunityResponse::Error { message, .. } => {
            Err(format!("server rejected get unread counts: {message}"))
        }
        other => Err(format!(
            "unexpected server response for GetUnreadCounts: {other:?}"
        )),
    }
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
    require_manage_community(&state, &community_id)?;
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
    require_manage_community(&state, &community_id)?;
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

    let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
        rekindle_protocol::dht::community::envelope::ControlPayload::SubmitOnboardingAnswers {
            answers,
        },
    );
    send_to_coordinator(state.inner(), &community_id, envelope).await
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

/// Check that the current user has MANAGE_COMMUNITY or ADMINISTRATOR permission.
fn require_manage_community(state: &SharedState, community_id: &str) -> Result<(), String> {
    use rekindle_protocol::dht::community::permissions_v2::Permissions;
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
    if perms.contains(Permissions::MANAGE_COMMUNITY) || perms.contains(Permissions::ADMINISTRATOR) {
        Ok(())
    } else {
        Err("missing MANAGE_COMMUNITY permission".into())
    }
}
