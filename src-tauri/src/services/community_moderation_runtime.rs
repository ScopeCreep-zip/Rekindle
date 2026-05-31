//! Phase 23.C — message-deletion runtime orchestration lifted from
//! `commands/community/moderation.rs`. Same pattern as the other
//! `services/community_*_runtime.rs` modules: legitimate Tauri-runtime
//! glue (write governance entry + gossip broadcast + SQLite delete +
//! UI emit), no CRDT merge or sig verify in these bodies.
//!
//! Both `admin_delete_channel_message` and `bulk_delete_channel_messages`
//! call `admin_delete_one_message` for the per-id work — pre-Phase-23 the
//! two handlers had near-duplicate inline bodies. Consolidating here
//! shrinks the file and gives a single place for future cross-cutting
//! changes (e.g. audit-chain entry on every delete).

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use crate::commands::community::helpers::require_permission;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

pub const BULK_DELETE_CAP: usize = 100;

pub async fn admin_delete_channel_message_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    channel_id: String,
    message_id: String,
    reason: Option<String>,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_MESSAGES)?;
    let owner_key = state_helpers::current_owner_key(state)?;
    admin_delete_one_message(
        state,
        pool,
        &community_id,
        &channel_id,
        &message_id,
        &owner_key,
        reason,
    )
    .await
}

pub fn message_id_to_bytes(message_id: &str) -> [u8; 16] {
    let stripped = message_id.strip_prefix("msg_").unwrap_or(message_id);
    uuid::Uuid::parse_str(stripped)
        .map(|u| *u.as_bytes())
        .unwrap_or([0u8; 16])
}

pub async fn purge_local_message(
    pool: &DbPool,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> Result<usize, String> {
    let owner = owner_key.to_string();
    let chan = channel_id.to_string();
    let mid = message_id.to_string();
    db_call(pool, move |conn| {
        let n = conn.execute(
            "DELETE FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
             AND conversation_type = 'channel' AND message_id = ?3",
            rusqlite::params![owner, chan, mid],
        )?;
        Ok(n)
    })
    .await
}

pub fn emit_message_deleted_local(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
) {
    if let Some(app_handle) = state_helpers::app_handle(state) {
        crate::event_dispatch::emit_live(
            &app_handle,
            "community-event",
            &crate::channels::CommunityEvent::MessageDeleted {
                community_id: community_id.to_string(),
                channel_id: channel_id.to_string(),
                message_id: message_id.to_string(),
            },
        );
    }
}

/// One admin-delete: write governance `AdminDelete` entry, gossip
/// `ControlPayload::MessageDeleted`, purge local SQLite row, emit
/// `CommunityEvent::MessageDeleted` locally. Returns `Ok(())` on
/// write_entry success even if gossip/purge fail (they log).
pub async fn admin_delete_one_message(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    owner_key: &str,
    reason: Option<String>,
) -> Result<(), String> {
    use crate::commands::community::helpers::hex_to_id_16;

    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::AdminDelete {
            message_id: message_id_to_bytes(message_id),
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(channel_id)),
            reason: reason.clone(),
            lamport,
        },
    )
    .await?;

    let _ = crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MessageDeleted {
            channel_id: channel_id.to_string(),
            message_id: message_id.to_string(),
        }),
    );

    if let Err(e) = purge_local_message(pool, owner_key, channel_id, message_id).await {
        tracing::warn!(
            community = %community_id,
            channel = %channel_id,
            %message_id,
            error = %e,
            "admin delete: local SQLite purge failed",
        );
    }
    emit_message_deleted_local(state, community_id, channel_id, message_id);
    Ok(())
}

pub async fn remove_community_member_inner(
    state: std::sync::Arc<crate::state::AppState>,
    pool: crate::db::DbPool,
    community_id: String,
    pseudonym_key: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::require_permission;
    use crate::db_helpers::db_call;
    use crate::services::community_registry_slot::clear_registry_presence_slot;
    use crate::state_helpers;
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    let owner_key = state_helpers::current_owner_key(&state)?;
    require_permission(&state, &community_id, Permissions::KICK_MEMBERS)?;

    crate::services::community::send_to_mesh(
        &state,
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::Kick {
            target_pseudonym: pseudonym_key.clone(),
        }),
    )?;

    if let Err(e) = clear_registry_presence_slot(&state, &pool, &community_id, &pseudonym_key).await
    {
        tracing::debug!(
            community = %community_id,
            member = %pseudonym_key,
            error = %e,
            "failed to clear kicked member registry slot"
        );
    }

    crate::services::community::analytics::log_member_leave(
        &pool,
        &owner_key,
        &community_id,
        &pseudonym_key,
    );

    let community_id_clone = community_id.clone();
    let pseudonym_key_clone = pseudonym_key.clone();
    db_call(&pool, move |conn| {
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

pub async fn timeout_member_inner(
    state: &crate::state::SharedState,
    pool: &crate::db::DbPool,
    community_id: String,
    pseudonym_key: String,
    duration_seconds: u64,
    reason: Option<String>,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_pseudo_32, require_permission};
    use crate::db_helpers::db_call;
    use crate::state_helpers;
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    require_permission(state, &community_id, Permissions::MODERATE_MEMBERS)?;
    let owner_key = state_helpers::current_owner_key(state)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
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

    let timeout_until = crate::db::timestamp_now() / 1000 + duration_seconds.cast_signed();
    let cid = community_id.clone();
    let pk = pseudonym_key.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE community_members SET timeout_until = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![timeout_until, owner_key, cid, pk],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

pub async fn remove_timeout_inner(
    state: &crate::state::SharedState,
    pool: &crate::db::DbPool,
    community_id: String,
    pseudonym_key: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_pseudo_32, require_permission};
    use crate::db_helpers::db_call;
    use crate::state_helpers;
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    require_permission(state, &community_id, Permissions::MODERATE_MEMBERS)?;
    let owner_key = state_helpers::current_owner_key(state)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::RemoveTimeoutEntry {
            target: rekindle_types::id::PseudonymKey(hex_to_pseudo_32(&pseudonym_key)),
            lamport,
        },
    )
    .await?;

    let cid = community_id.clone();
    let pk = pseudonym_key.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE community_members SET timeout_until = NULL WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![owner_key, cid, pk],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches PermissionOverwrite shape"
)]
pub async fn set_channel_overwrite_inner(
    state: &crate::state::SharedState,
    pool: &crate::db::DbPool,
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    allow: u64,
    deny: u64,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use crate::db_helpers::db_call;
    use crate::state_helpers;
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    require_permission(state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let owner_key = state_helpers::current_owner_key(state)?;

    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
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
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO channel_overwrites (owner_key, community_id, channel_id, target_type, target_id, allow, deny) VALUES (?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![owner_key, comm_id, chan_id, tgt_type, tgt_id, allow.cast_signed(), deny.cast_signed()],
        )?;
        Ok(())
    }).await?;
    Ok(())
}

pub async fn delete_channel_overwrite_inner(
    state: &crate::state::SharedState,
    pool: &crate::db::DbPool,
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_id_16, require_permission};
    use crate::db_helpers::db_call;
    use crate::state_helpers;
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    require_permission(state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let owner_key = state_helpers::current_owner_key(state)?;

    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
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
    db_call(pool, move |conn| {
        conn.execute(
            "DELETE FROM channel_overwrites WHERE owner_key = ? AND community_id = ? AND channel_id = ? AND target_type = ? AND target_id = ?",
            rusqlite::params![owner_key, comm_id, chan_id, tgt_type, tgt_id],
        )?;
        Ok(())
    }).await?;
    Ok(())
}

pub async fn set_slowmode_inner(
    state: &crate::state::SharedState,
    community_id: String,
    channel_id: String,
    seconds: u32,
) -> Result<(), String> {
    use crate::commands::community::helpers::hex_to_id_16;
    use crate::state_helpers;

    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: None,
            topic: None,
            forum_tags: None,
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

pub async fn ban_member_inner(
    state: &crate::state::SharedState,
    community_id: String,
    pseudonym_key: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_pseudo_32, require_permission};
    use crate::state_helpers;
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    require_permission(state, &community_id, Permissions::BAN_MEMBERS)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::BanEntry {
            target: rekindle_types::id::PseudonymKey(hex_to_pseudo_32(&pseudonym_key)),
            reason: None,
            lamport,
        },
    )
    .await?;

    if let Some(app_handle) = state_helpers::app_handle(state) {
        let state = state.clone();
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

pub async fn unban_member_inner(
    state: &crate::state::SharedState,
    community_id: String,
    pseudonym_key: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::{hex_to_pseudo_32, require_permission};
    use crate::state_helpers;
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    require_permission(state, &community_id, Permissions::BAN_MEMBERS)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
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
