use std::sync::Arc;

use rekindle_codec::community::channel_record::{
    decode_channel_entries, ChannelMessage, ChannelRecordEntry,
};
use rekindle_records::retry;

use crate::channels::ChatEvent;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

#[derive(Clone)]
pub struct PendingMessageFetch {
    pub community_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub subkey_index: u32,
    pub sequence: u64,
    pub content_hash: String,
    pub attempt: u32,
}

pub(super) fn verify_notification_message(
    pending: &PendingMessageFetch,
    message: &ChannelMessage,
) -> Result<(), &'static str> {
    rekindle_channel::verify_message_content_hash(&pending.content_hash, message)
}

pub(super) fn emit_message_received(
    state: &Arc<AppState>,
    pending: &PendingMessageFetch,
    from: String,
    body: String,
    timestamp: u64,
    decryption_failed: bool,
    automod_blurred: bool,
) {
    let event = ChatEvent::MessageReceived {
        from,
        body,
        decryption_failed,
        automod_blurred,
        timestamp,
        conversation_id: pending.channel_id.clone(),
        server_message_id: Some(pending.message_id.clone()),
        reply_to_id: None,
        sender_display_name: None,
    };
    // Journaled so a community window that reloads mid-stream gets it.
    crate::event_dispatch::emit_from_state(
        state,
        crate::event_dispatch::WebviewEvent::ChannelChat {
            community_id: Some(pending.community_id.clone()),
            event,
        },
    );
}

pub(super) async fn message_exists(
    pool: &Db,
    owner_key: &str,
    message_id: &str,
) -> Result<bool, String> {
    use rusqlite::OptionalExtension;
    let owner = owner_key.to_string();
    let mid = message_id.to_string();
    db_call(pool, move |conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM messages WHERE owner_key = ?1 AND message_id = ?2 LIMIT 1",
                rusqlite::params![owner, mid],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    })
    .await
}

/// Result of fetching a channel notification target — either a regular message
/// or a forward (which carries an `original_author` for attribution).
pub(super) struct FetchedChannelEntry {
    pub message: ChannelMessage,
    /// The channel record the entry was read from — part of its AAD.
    pub record_key: String,
    /// `Some(pseudonym_hex)` when the entry came from a `ChannelRecordEntry::Forward`.
    pub forwarded_from_author: Option<String>,
}

pub(super) async fn fetch_channel_message(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    subkey_index: u32,
    message_id: &str,
) -> Result<FetchedChannelEntry, String> {
    // The channel's one record: its genesis message record.
    let record_key = state
        .communities
        .read()
        .get(community_id)
        .and_then(|c| c.channel_log_keys.get(channel_id).cloned())
        .ok_or_else(|| "channel record key not found".to_string())?;
    let pool = state_helpers::record_pool(state)?;
    let value = pool
        .read_once(&record_key, subkey_index, true)
        .await
        .map_err(|e| format!("get_dht_value failed: {e}"))?
        .ok_or_else(|| "message id not found in the channel record".to_string())?;
    let entries = decode_channel_entries(&value.data)
        .map_err(|e| format!("invalid channel page payload: {e}"))?;
    entries
        .into_iter()
        .find_map(|entry| match entry {
            ChannelRecordEntry::Message(message)
                if message.message_id.as_deref() == Some(message_id) =>
            {
                Some(FetchedChannelEntry {
                    message,
                    record_key: record_key.clone(),
                    forwarded_from_author: None,
                })
            }
            ChannelRecordEntry::Forward(forward)
                if forward.message_id.as_deref() == Some(message_id) =>
            {
                let original_author = forward.original_author.clone();
                Some(FetchedChannelEntry {
                    message: ChannelMessage {
                        sequence: forward.sequence,
                        sender_pseudonym: forward.sender_pseudonym,
                        ciphertext: forward.content_snapshot,
                        mek_generation: forward.mek_generation,
                        timestamp: forward.timestamp,
                        reply_to: None,
                        lamport_ts: forward.lamport_ts,
                        message_id: forward.message_id,
                        attachment: None,
                        // Forwarded messages don't carry the original
                        // sender's mention metadata across — the
                        // recipient is being shown the snapshot, not
                        // re-pinged. Leave flags + lists empty so
                        // notification routing treats the forward as a
                        // normal (non-mention) message.
                        flags: 0,
                        mentioned_pseudonyms: Vec::new(),
                        mentioned_roles: Vec::new(),
                    },
                    record_key: record_key.clone(),
                    forwarded_from_author: Some(original_author),
                })
            }
            _ => None,
        })
        .ok_or_else(|| "message id not found in the channel record".to_string())
}

pub(super) fn update_peer_sequence(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    sequence: u64,
) {
    if sequence == 0 {
        return;
    }
    let key = (sender_pseudonym.to_string(), channel_id.to_string());
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.peer_sequences.insert(key, sequence);
    }
}

pub fn queue_message_fetch_retry(state: Arc<AppState>, pending: PendingMessageFetch) {
    crate::state_helpers::spawn_in_login_with_token(
        &state.clone(),
        "message fetch retry",
        |stop| async move {
            let backoff = tokio::time::sleep(retry::backoff_duration(pending.attempt));
            if stop.run_until_cancelled(backoff).await.is_none() {
                return;
            }
            if let Some(app_handle) = state_helpers::app_handle(&state) {
                let _ = super::message_notifications_handle::handle_message_notification(
                    &app_handle,
                    &state,
                    PendingMessageFetch {
                        attempt: pending.attempt + 1,
                        ..pending
                    },
                )
                .await;
            }
        },
    );
}
