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

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::permissions;

use crate::commands::community::helpers::require_permission;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;

pub const BULK_DELETE_CAP: usize = 100;

pub async fn admin_delete_channel_message_inner(
    state: &SharedState,
    pool: &Db,
    community_id: String,
    channel_id: String,
    message_id: String,
    reason: Option<String>,
) -> Result<(), String> {
    require_permission(state, &community_id, permissions::MANAGE_MESSAGES)?;
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
    pool: &Db,
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
        crate::event_dispatch::emit_subscription(
            &app_handle,
            &rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
                rekindle_types::subscription_events::ChannelMessageEvent::Deleted {
                    community: community_id.to_string(),
                    channel: channel_id.to_string(),
                    message_id: message_id.to_string(),
                },
            ),
        );
    }
}

/// One admin-delete: write governance `AdminDelete` entry, gossip
/// `ControlPayload::MessageDeleted`, purge local SQLite row, emit
/// `CommunityEvent::MessageDeleted` locally. Returns `Ok(())` on
/// write_entry success even if gossip/purge fail (they log).
pub async fn admin_delete_one_message(
    state: &SharedState,
    pool: &Db,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    owner_key: &str,
    reason: Option<String>,
) -> Result<(), String> {
    use crate::commands::community::helpers::hex_to_id_16;

    let lamport =
        state_helpers::next_governance_lamport(state, community_id).map_err(|e| e.to_string())?;
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

pub async fn set_slowmode_inner(
    state: &crate::state::SharedState,
    community_id: String,
    channel_id: String,
    seconds: u32,
) -> Result<(), String> {
    use crate::commands::community::helpers::hex_to_id_16;
    use crate::state_helpers;

    let lamport =
        state_helpers::next_governance_lamport(state, &community_id).map_err(|e| e.to_string())?;
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
