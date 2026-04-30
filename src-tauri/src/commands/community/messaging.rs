use tauri::State;

use crate::commands::chat::Message;
use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::require_permission;
use super::legacy::{load_channel_messages_from_smpl, merge_message_lists};
use super::types::SendChannelMessageResponse;

/// Send a message in a community channel.
#[tauri::command]
pub async fn send_channel_message(
    channel_id: String,
    body: String,
    reply_to_id: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<SendChannelMessageResponse, String> {
    let sent =
        crate::services::community::send_message(state.inner(), pool.inner(), &channel_id, &body)
            .await?;
    crate::services::community::emit_local_chat_event(&app, &sent, &channel_id);
    let _ = reply_to_id;
    tracing::info!(status = %sent.result.status, message_id = %sent.result.message_id, "channel message sent");
    Ok(SendChannelMessageResponse {
        status: sent.result.status,
        message_id: sent.result.message_id,
    })
}

#[tauri::command]
pub async fn edit_channel_message(
    channel_id: String,
    message_id: String,
    new_body: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool;
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
        state.inner(),
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

    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::MessageDeleted {
            channel_id,
            message_id,
        }),
    )
}

#[tauri::command]
pub async fn get_channel_messages(
    channel_id: String,
    limit: u32,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    let our_key = state_helpers::current_owner_key(state.inner()).unwrap_or_default();

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
        require_permission(state.inner(), cid, Permissions::READ_MESSAGE_HISTORY)?;
    }

    let channel_id_clone = channel_id.clone();
    let ok = our_key.clone();
    let mpk = my_pseudonym_key.clone();
    let mut messages: Vec<Message> = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, automod_blurred, timestamp, message_id FROM messages \
                 WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
                 ORDER BY COALESCE(NULLIF(lamport_ts, 0), timestamp) DESC, sender_key DESC LIMIT ?",
        )?;

        let rows = stmt.query_map(rusqlite::params![ok, channel_id_clone, limit], |row| {
            let sender = db::get_str(row, "sender_key");
            let is_own = sender == ok || sender == mpk;
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
        let smpl_messages = load_channel_messages_from_smpl(
            state.inner(),
            pool.inner(),
            cid,
            &channel_id,
            None,
            limit,
        )
        .await?;
        merge_message_lists(&mut messages, smpl_messages, limit);
    }

    let _ = (community_id, app);
    Ok(messages)
}

#[tauri::command]
pub async fn get_older_channel_messages(
    community_id: String,
    channel_id: String,
    before_timestamp: u64,
    limit: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    require_permission(
        state.inner(),
        &community_id,
        Permissions::READ_MESSAGE_HISTORY,
    )?;
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
    let mut messages: Vec<Message> = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, automod_blurred, timestamp, message_id FROM messages \
             WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
             AND timestamp < ? \
             ORDER BY COALESCE(NULLIF(lamport_ts, 0), timestamp) DESC, sender_key DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![ok, channel_id_clone, before_ts, limit],
            |row| {
                let sender = db::get_str(row, "sender_key");
                let is_own = sender == ok || sender == mpk;
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
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
        Some(before_timestamp),
        limit,
    )
    .await?;
    merge_message_lists(&mut messages, smpl_messages, limit);
    Ok(messages)
}
