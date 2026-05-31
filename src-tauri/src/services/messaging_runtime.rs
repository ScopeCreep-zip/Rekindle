//! Phase 23.C — channel-messaging Tauri-runtime orchestration lifted
//! from `commands/community/messaging.rs`. Hosts `get_channel_messages`
//! body — local SQLite scan + SMPL fetch merge — so the Tauri handler
//! stays a thin delegation.

use std::sync::Arc;

use crate::channel_materialize::load_channel_messages_from_smpl;
use crate::commands::chat::Message;
use crate::commands::community::helpers::require_permission;
use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::message_view::merge_message_lists;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageResponse {
    pub status: String,
    pub message_id: String,
}

pub async fn get_channel_messages_inner(
    state: Arc<AppState>,
    pool: DbPool,
    channel_id: String,
    limit: u32,
) -> Result<Vec<Message>, String> {
    let our_key = state_helpers::current_owner_key(&state).unwrap_or_default();

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

    let channel_id_clone = channel_id.clone();
    let ok = our_key.clone();
    let mpk = my_pseudonym_key.clone();
    let mut messages: Vec<Message> = db_call(&pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, automod_blurred, timestamp, message_id, forwarded_from_author, attachment_json, flags \
                 FROM messages \
                 WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
                 ORDER BY COALESCE(NULLIF(lamport_ts, 0), timestamp) DESC, sender_key DESC LIMIT ?",
        )?;

        let rows = stmt.query_map(rusqlite::params![ok, channel_id_clone, limit], |row| {
            let sender = db::get_str(row, "sender_key");
            let is_own = sender == ok || sender == mpk;
            let attachment = db::get_str_opt(row, "attachment_json")
                .and_then(|json| serde_json::from_str::<crate::commands::chat::MessageAttachmentDto>(&json).ok());
            let flags = u32::try_from(row.get::<_, i64>("flags").unwrap_or(0).max(0)).unwrap_or(0);
            Ok(Message {
                id: db::get_i64(row, "id"),
                sender_id: sender,
                body: db::get_str(row, "body"),
                decryption_failed: false,
                automod_blurred: row.get::<_, i64>("automod_blurred").unwrap_or(0) != 0,
                timestamp: db::get_i64(row, "timestamp"),
                is_own,
                server_message_id: db::get_str_opt(row, "message_id"),
                reactions: None,
                pinned: None,
                poll: None,
                forwarded_from_author: db::get_str_opt(row, "forwarded_from_author"),
                attachment,
                flags,
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    })
    .await?;

    messages.reverse();

    tracing::debug!(
        owner_key = %our_key,
        channel_id = %channel_id,
        local_count = messages.len(),
        "loaded channel messages from local DB"
    );

    if let Some(ref cid) = community_id {
        let smpl_messages =
            load_channel_messages_from_smpl(&state, &pool, cid, &channel_id, None, limit).await?;
        merge_message_lists(&mut messages, smpl_messages, limit);
    }

    Ok(messages)
}

pub fn edit_channel_message_inner(
    state: &Arc<AppState>,
    channel_id: String,
    message_id: String,
    new_body: &str,
) -> Result<(), String> {
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

    crate::services::community::send_to_mesh(
        state,
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::MessageEdited {
            channel_id,
            message_id,
            new_ciphertext,
            mek_generation,
            edited_at: rekindle_utils::timestamp_secs(),
        }),
    )
}

pub fn delete_channel_message_inner(
    state: &Arc<AppState>,
    channel_id: String,
    message_id: String,
) -> Result<(), String> {
    let community_id = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .map(|c| c.id.clone())
            .ok_or("channel not found in any community")?
    };

    crate::services::community::send_to_mesh(
        state,
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::MessageDeleted {
            channel_id,
            message_id,
        }),
    )
}

#[allow(clippy::too_many_arguments, reason = "Tauri command surface — matches forward_channel_message args")]
pub async fn forward_channel_message_inner(
    state: &Arc<AppState>,
    pool: &DbPool,
    app: &tauri::AppHandle,
    source_community_id: String,
    source_channel_id: String,
    source_message_id: String,
    dest_community_id: String,
    dest_channel_id: String,
) -> Result<SendChannelMessageResponse, String> {
    let sent = crate::services::community::channel_messages::forward_message(
        state,
        pool,
        &source_community_id,
        &source_channel_id,
        &source_message_id,
        &dest_community_id,
        &dest_channel_id,
    )
    .await?;
    crate::services::community::emit_local_chat_event(app, &sent, &dest_channel_id);
    Ok(SendChannelMessageResponse {
        status: sent.result.status,
        message_id: sent.result.message_id,
    })
}

pub async fn send_channel_message_inner(
    state: &Arc<AppState>,
    pool: &DbPool,
    app: &tauri::AppHandle,
    channel_id: String,
    body: String,
) -> Result<SendChannelMessageResponse, String> {
    let sent =
        crate::services::community::send_message(state, pool, &channel_id, &body).await?;
    crate::services::community::emit_local_chat_event(app, &sent, &channel_id);
    tracing::info!(status = %sent.result.status, message_id = %sent.result.message_id, "channel message sent");
    Ok(SendChannelMessageResponse {
        status: sent.result.status,
        message_id: sent.result.message_id,
    })
}

pub async fn get_older_channel_messages_inner(
    state: Arc<AppState>,
    pool: DbPool,
    community_id: String,
    channel_id: String,
    before_timestamp: u64,
    limit: u32,
) -> Result<Vec<Message>, String> {
    require_permission(&state, &community_id, Permissions::READ_MESSAGE_HISTORY)?;
    let our_key = state_helpers::current_owner_key(&state).unwrap_or_default();
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
    let mut messages: Vec<Message> = db_call(&pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, automod_blurred, timestamp, message_id, forwarded_from_author, attachment_json, flags \
             FROM messages \
             WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
             AND timestamp < ? \
             ORDER BY COALESCE(NULLIF(lamport_ts, 0), timestamp) DESC, sender_key DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![ok, channel_id_clone, before_ts, limit],
            |row| {
                let sender = db::get_str(row, "sender_key");
                let is_own = sender == ok || sender == mpk;
                let attachment = db::get_str_opt(row, "attachment_json")
                    .and_then(|json| serde_json::from_str::<crate::commands::chat::MessageAttachmentDto>(&json).ok());
                let flags = u32::try_from(row.get::<_, i64>("flags").unwrap_or(0).max(0)).unwrap_or(0);
                Ok(Message {
                    id: db::get_i64(row, "id"),
                    sender_id: sender,
                    body: db::get_str(row, "body"),
                    decryption_failed: false,
                    automod_blurred: row.get::<_, i64>("automod_blurred").unwrap_or(0) != 0,
                    timestamp: db::get_i64(row, "timestamp"),
                    is_own,
                    server_message_id: db::get_str_opt(row, "message_id"),
                    reactions: None,
                    pinned: None,
                    poll: None,
                    forwarded_from_author: db::get_str_opt(row, "forwarded_from_author"),
                    attachment,
                    flags,
                })
            },
        )?;
        let mut msgs = Vec::new();
        for row in rows {
            msgs.push(row?);
        }
        Ok(msgs)
    })
    .await?;

    messages.reverse();
    let smpl_messages = load_channel_messages_from_smpl(
        &state,
        &pool,
        &community_id,
        &channel_id,
        Some(before_timestamp),
        limit,
    )
    .await?;
    merge_message_lists(&mut messages, smpl_messages, limit);
    Ok(messages)
}
