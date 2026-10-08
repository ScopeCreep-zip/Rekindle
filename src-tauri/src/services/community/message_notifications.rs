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
    crate::event_dispatch::emit_journaled(
        state,
        crate::event_dispatch::WebviewEvent::ChannelChat {
            community_id: Some(pending.community_id.clone()),
            event,
        },
    );
}

/// Open a fetched message body under exactly its generation of the
/// channel's text key, bound to the record and subkey it was fetched from
/// (see `channel_materialize::decrypt_channel_record_message`). `None`
/// when that generation is not held or the body does not open there.
pub(super) fn decrypt_message_body(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    fetched: &FetchedChannelEntry,
    subkey_index: u32,
) -> Option<String> {
    let opened = crate::channel_materialize::decrypt_channel_record_message(
        state,
        community_id,
        channel_id,
        fetched.message.mek_generation,
        &fetched.message.ciphertext,
        rekindle_secrets::channel_body::BodyPosition {
            channel_record_key: &fetched.record_key,
            subkey_index,
            lamport_ts: fetched.message.lamport_ts,
        },
    );
    (!opened.decryption_failed).then_some(opened.body)
}

pub(super) async fn message_exists(pool: &Db, owner_key: &str, message_id: &str) -> bool {
    let owner = owner_key.to_string();
    let mid = message_id.to_string();
    db_call(pool, move |conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM messages WHERE owner_key = ?1 AND message_id = ?2 LIMIT 1",
                rusqlite::params![owner, mid],
                |_| Ok(()),
            )
            .is_ok())
    })
    .await
    .unwrap_or(false)
}

/// Result of fetching a channel notification target — either a regular message
/// or a forward (which carries an `original_author` for attribution).
pub(super) struct FetchedChannelEntry {
    pub message: ChannelMessage,
    /// The segment record the entry was read from — part of its AAD.
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
    // Plate Gate (architecture §15.4): a channel may have one SMPL record
    // per segment that contains a writer. Scan each segment's record at
    // the given subkey looking for the message_id. Genesis segment 0 is
    // always present; segment-N records are populated lazily via
    // `ChannelSegmentLinked` governance entries.
    let segment_records = crate::services::community::segments::channel_record_keys_per_segment(
        state,
        community_id,
        channel_id,
    );
    if segment_records.is_empty() {
        return Err("channel record key not found".into());
    }
    let pool = state_helpers::record_pool(state)?;
    let mut last_error: Option<String> = None;
    for (_segment_index, record_key_str) in segment_records {
        let record_key = match record_key_str.parse::<veilid_core::RecordKey>() {
            Ok(key) => key,
            Err(e) => {
                last_error = Some(format!("invalid channel record key: {e}"));
                continue;
            }
        };
        let value = match pool.read_once(&record_key, subkey_index, true).await {
            Ok(Some(v)) => v,
            Ok(None) => {
                continue;
            }
            Err(e) => {
                last_error = Some(format!("get_dht_value failed: {e}"));
                continue;
            }
        };
        let entries = match decode_channel_entries(value.data()) {
            Ok(entries) => entries,
            Err(e) => {
                last_error = Some(format!("invalid channel page payload: {e}"));
                continue;
            }
        };
        if let Some(found) = entries.into_iter().find_map(|entry| match entry {
            ChannelRecordEntry::Message(message)
                if message.message_id.as_deref() == Some(message_id) =>
            {
                Some(FetchedChannelEntry {
                    message,
                    record_key: record_key_str.clone(),
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
                    record_key: record_key_str.clone(),
                    forwarded_from_author: Some(original_author),
                })
            }
            _ => None,
        }) {
            return Ok(found);
        }
    }
    Err(last_error.unwrap_or_else(|| "message id not found in any segment record".into()))
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

pub(super) fn emit_automod_alert(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    rule_name: &str,
) {
    let can_moderate = rekindle_governance::permissions::has_moderation_capability(
        state_helpers::my_permissions(state, community_id, None),
    );
    if can_moderate {
        crate::event_dispatch::emit_subscription(
            app_handle,
            &rekindle_types::subscription_events::SubscriptionEvent::System(
                rekindle_types::subscription_events::SystemEvent::AutoModAlert {
                    community: community_id.to_string(),
                    channel: channel_id.to_string(),
                    message_id: message_id.to_string(),
                    rule_name: rule_name.to_string(),
                },
            ),
        );
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
