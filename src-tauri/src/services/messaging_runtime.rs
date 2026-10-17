//! Phase 23.C — channel-messaging Tauri-runtime orchestration lifted
//! from `commands/community/messaging.rs`. Hosts `get_channel_messages`
//! body — local SQLite scan + SMPL fetch merge — so the Tauri handler
//! stays a thin delegation.

use std::sync::Arc;

use crate::channel_materialize::load_channel_messages_from_smpl;
use crate::commands::chat::Message;
use crate::commands::community::helpers::require_permission;
use crate::db_helpers::db_call;
use crate::message_view::merge_message_lists;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_db::row::{get, get_json_opt};
use rekindle_db::Db;
use rekindle_types::permissions;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageResponse {
    pub status: String,
    pub message_id: String,
}

pub async fn get_channel_messages_inner(
    state: Arc<AppState>,
    pool: Db,
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
        require_permission(&state, cid, permissions::READ_HISTORY)?;
    }

    let channel_id_clone = channel_id.clone();
    let ok = our_key.clone();
    let mpk = my_pseudonym_key.clone();
    let mut messages: Vec<Message> = db_call(&pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, automod_blurred, timestamp, message_id, forwarded_from_author, attachment_json, flags, send_state \
                 FROM messages \
                 WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
                 ORDER BY COALESCE(NULLIF(lamport_ts, 0), timestamp) DESC, sender_key DESC LIMIT ?",
        )?;

        let rows = stmt.query_map(rusqlite::params![ok, channel_id_clone, limit], |row| {
            channel_message_from_row(row, &ok, &mpk)
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
    let community_id = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .map(|c| c.id.clone())
            .ok_or("channel not found in any community")?
    };

    // Sealed under the channel's current text key and stamped with that
    // key's generation, which the receiver opens under exactly.
    let scope =
        state_helpers::text_scope(state, &community_id, &channel_id).ok_or("not a channel id")?;
    let (epoch, key) = rekindle_types::channel_keys::current_key(
        &*state_helpers::key_provider(state),
        &community_id,
        scope,
    )
    .ok_or("MEK not available")?;
    let mek_generation = epoch.0;
    let new_ciphertext =
        rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(*key, mek_generation)
            .encrypt(new_body.as_bytes())
            .map_err(|e| format!("MEK encryption failed: {e}"))?;

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

pub async fn forward_channel_message_inner(
    state: &Arc<AppState>,
    pool: &Db,
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
    Ok(SendChannelMessageResponse {
        status: sent.status,
        message_id: sent.message_id,
    })
}

pub async fn send_channel_message_inner(
    state: &Arc<AppState>,
    pool: &Db,
    channel_id: String,
    body: String,
) -> Result<SendChannelMessageResponse, String> {
    let sent = crate::services::community::send_message(state, pool, &channel_id, &body).await?;
    tracing::info!(status = %sent.status, message_id = %sent.message_id, "channel message sent");
    Ok(SendChannelMessageResponse {
        status: sent.status,
        message_id: sent.message_id,
    })
}

pub async fn get_older_channel_messages_inner(
    state: Arc<AppState>,
    pool: Db,
    community_id: String,
    channel_id: String,
    before_timestamp: u64,
    limit: u32,
) -> Result<Vec<Message>, String> {
    require_permission(&state, &community_id, permissions::READ_HISTORY)?;
    let our_key = state_helpers::current_owner_key(&state).unwrap_or_default();
    let my_pseudonym_key =
        state_helpers::my_pseudonym_key(&state, &community_id).unwrap_or_default();

    let channel_id_clone = channel_id.clone();
    let ok = our_key.clone();
    let mpk = my_pseudonym_key.clone();
    let before_ts = before_timestamp.cast_signed();
    let mut messages: Vec<Message> = db_call(&pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, automod_blurred, timestamp, message_id, forwarded_from_author, attachment_json, flags, send_state \
             FROM messages \
             WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
             AND timestamp < ? \
             ORDER BY COALESCE(NULLIF(lamport_ts, 0), timestamp) DESC, sender_key DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![ok, channel_id_clone, before_ts, limit],
            |row| channel_message_from_row(row, &ok, &mpk),
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

/// One `messages` row of a channel, read strictly (plan D2). `own_key` and
/// `pseudonym_key` mark which senders are us.
fn channel_message_from_row(
    row: &rusqlite::Row<'_>,
    own_key: &str,
    pseudonym_key: &str,
) -> rusqlite::Result<Message> {
    let sender: String = get(row, "sender_key")?;
    let is_own = sender == own_key || sender == pseudonym_key;
    let attachment = get_json_opt::<rekindle_files::AttachmentRecordJson>(row, "attachment_json")?
        .map(crate::commands::chat::MessageAttachmentDto::from_record);
    Ok(Message {
        id: get(row, "id")?,
        sender_id: sender,
        body: get(row, "body")?,
        decryption_failed: false,
        automod_blurred: get::<i64>(row, "automod_blurred")? != 0,
        timestamp: get(row, "timestamp")?,
        is_own,
        server_message_id: get(row, "message_id")?,
        reactions: None,
        pinned: None,
        poll: None,
        forwarded_from_author: get(row, "forwarded_from_author")?,
        attachment,
        flags: get(row, "flags")?,
        send_state: crate::commands::chat::stored_send_state(row)?,
    })
}
