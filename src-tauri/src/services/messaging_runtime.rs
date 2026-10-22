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
use rekindle_db::Db;
use rekindle_types::permissions;

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
        rekindle_db::repo::messages::channel::page(conn, &ok, &channel_id_clone, None, limit)
    })
    .await?
    .into_iter()
    .map(|row| channel_message(row, &our_key, &mpk))
    .collect();

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
        rekindle_db::repo::messages::channel::page(
            conn,
            &ok,
            &channel_id_clone,
            Some(before_ts),
            limit,
        )
    })
    .await?
    .into_iter()
    .map(|row| channel_message(row, &our_key, &mpk))
    .collect();
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

/// One stored channel message as the window renders it. `own_key` and
/// `pseudonym_key` mark which senders are us.
fn channel_message(
    row: rekindle_db::repo::messages::channel::StoredChannelMessage,
    own_key: &str,
    pseudonym_key: &str,
) -> Message {
    let is_own = row.sender_key == own_key || row.sender_key == pseudonym_key;
    let attachment = row
        .attachment_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<rekindle_files::AttachmentRecordJson>(json).ok())
        .map(crate::commands::chat::MessageAttachmentDto::from_record);
    Message {
        id: row.id,
        sender_id: row.sender_key,
        body: row.body,
        decryption_failed: false,
        automod_blurred: row.automod_blurred,
        timestamp: row.timestamp,
        is_own,
        server_message_id: row.message_id,
        reactions: None,
        pinned: None,
        poll: None,
        forwarded_from_author: row.forwarded_from_author,
        attachment,
        flags: row.flags,
        send_state: row.send_state,
    }
}
