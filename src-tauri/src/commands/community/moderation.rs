use tauri::State;

use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::{hex_to_id_16, hex_to_pseudo_32, require_permission};
use super::legacy::clear_registry_presence_slot;
use super::types::BannedMemberInfo;

/// Remove a member from a community.
#[tauri::command]
pub async fn remove_community_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    require_permission(state.inner(), &community_id, Permissions::KICK_MEMBERS)?;

    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::Kick {
            target_pseudonym: pseudonym_key.clone(),
        }),
    )?;

    if let Err(e) =
        clear_registry_presence_slot(state.inner(), pool.inner(), &community_id, &pseudonym_key)
            .await
    {
        tracing::debug!(
            community = %community_id,
            member = %pseudonym_key,
            error = %e,
            "failed to clear kicked member registry slot"
        );
    }

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
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::TimeoutEntry {
            target: rekindle_types::id::PseudonymKey(hex_to_pseudo_32(&pseudonym_key)),
            duration_seconds,
            reason,
            started_at: rekindle_utils::timestamp_secs(),
            lamport,
        },
    )
    .await?;

    let timeout_until = db::timestamp_now() / 1000 + duration_seconds.cast_signed();
    let cid = community_id.clone();
    let pk = pseudonym_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "UPDATE community_members SET timeout_until = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![timeout_until, owner_key, cid, pk],
        )?;
        Ok(())
    })
    .await?;
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
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::RemoveTimeoutEntry {
            target: rekindle_types::id::PseudonymKey(hex_to_pseudo_32(&pseudonym_key)),
            lamport,
        },
    )
    .await?;

    let cid = community_id.clone();
    let pk = pseudonym_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "UPDATE community_members SET timeout_until = NULL WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![owner_key, cid, pk],
        )?;
        Ok(())
    })
    .await?;
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
    require_permission(state.inner(), &community_id, Permissions::MANAGE_COMMUNITY)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::PermissionOverwrite {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            target_type: target_type.clone(),
            target_id: target_id.clone(),
            allow,
            deny,
            lamport,
        },
    )
    .await?;

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
    require_permission(state.inner(), &community_id, Permissions::MANAGE_COMMUNITY)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::PermissionOverwrite {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            target_type: target_type.clone(),
            target_id: target_id.clone(),
            allow: 0,
            deny: 0,
            lamport,
        },
    )
    .await?;

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
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: None,
            topic: None,
            position: None,
            slowmode_seconds: Some(seconds),
            nsfw: None,
            category_id: None,
            lamport,
        },
    )
    .await?;

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
            ch.slowmode_seconds = Some(seconds);
        }
    }
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
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::BanEntry {
            target: rekindle_types::id::PseudonymKey(hex_to_pseudo_32(&pseudonym_key)),
            reason: None,
            lamport,
        },
    )
    .await?;

    if let Some(app_handle) = state_helpers::app_handle(state.inner()) {
        let state = state.inner().clone();
        let community_id = community_id.clone();
        let pseudonym_key = pseudonym_key.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::services::community::rotate_text_mek_for_departure(
                &app_handle,
                &state,
                &community_id,
                &pseudonym_key,
            )
            .await
            {
                tracing::debug!(community = %community_id, member = %pseudonym_key, error = %error, "text MEK rotation skipped after ban");
            }
        });
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
    require_permission(state.inner(), &community_id, Permissions::BAN_MEMBERS)?;
    let _ = pool;
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::UnbanEntry {
            target: rekindle_types::id::PseudonymKey(hex_to_pseudo_32(&pseudonym_key)),
            lamport,
        },
    )
    .await?;

    tracing::info!(community = %community_id, member = %pseudonym_key, "member unbanned");
    Ok(())
}

/// Get the list of banned members for a community from the merged governance state.
#[tauri::command]
pub async fn get_ban_list(
    community_id: String,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<Vec<BannedMemberInfo>, String> {
    let communities = state.communities.read();
    let community = communities
        .get(&community_id)
        .ok_or("community not found")?;
    let mut bans: Vec<_> = community
        .governance_state
        .as_ref()
        .map(|gov| gov.bans.iter().cloned().collect())
        .unwrap_or_default();
    bans.sort_by_key(|a| hex::encode(a.0));

    Ok(bans
        .into_iter()
        .map(|pseudo| {
            let pseudonym_key = hex::encode(pseudo.0);
            BannedMemberInfo {
                display_name: if pseudonym_key.len() > 12 {
                    format!("{}…", &pseudonym_key[..12])
                } else {
                    pseudonym_key.clone()
                },
                pseudonym_key,
                banned_at: 0,
                reason: None,
                banned_by: String::new(),
            }
        })
        .collect())
}
