use std::sync::Arc;

use rekindle_protocol::dht::community::channel_record::{
    decode_channel_entries, ChannelMessage, ChannelRecordEntry,
};
use rekindle_records::retry;
use tauri::Manager;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

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
    app_handle: &tauri::AppHandle,
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
    // Phase 10 — journal + emit so a hard-quit mid-stream client can
    // resume from the last cursor it saw and have this community message
    // replayed on cold start.
    crate::event_dispatch::emit_journaled(app_handle, state, "chat-event", &event);
}

/// Resolve the channel's SMPL record key (string form) for AAD
/// reconstruction. Returns `None` when the channel hasn't been merged
/// from governance yet — callers fall back to the no-AAD path below
/// for backward-compat with messages written before §8 line 1626 was
/// implemented.
fn channel_record_key_for(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<String> {
    let communities = state.communities.read();
    communities
        .get(community_id)?
        .channels
        .iter()
        .find(|ch| ch.id == channel_id)?
        .message_record_key
        .clone()
}

pub(super) fn decrypt_message_body(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    pending: &PendingMessageFetch,
    message: &ChannelMessage,
) -> Option<String> {
    // Architecture §8 line 1626 — reconstruct the same AAD the sender
    // bound. If the SMPL record key isn't known yet, fall back to the
    // no-AAD path below for legacy messages written before AAD landed.
    let record_key = channel_record_key_for(state, community_id, channel_id);
    let aad_owned = record_key
        .as_ref()
        .map(|key| rekindle_crypto::group::media_key::ChannelAad {
            channel_record_key: key.as_bytes(),
            subkey_index: pending.subkey_index,
            lamport_ts: message.lamport_ts,
        });

    {
        let channel_mek_cache = state.channel_mek_cache.lock();
        if let Some(mek) =
            channel_mek_cache.get(&(community_id.to_string(), channel_id.to_string()))
        {
            if mek.generation() == message.mek_generation {
                if let Some(aad) = aad_owned {
                    if let Ok(bytes) = mek.decrypt_with_aad(&message.ciphertext, aad) {
                        return String::from_utf8(bytes).ok();
                    }
                }
                // Legacy fallback for messages written before AAD landed.
                return mek
                    .decrypt(&message.ciphertext)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok());
            }
        }
    }

    let mek_cache = state.mek_cache.lock();
    let decrypted = mek_cache
        .get(community_id)
        .filter(|mek| mek.generation() == message.mek_generation)
        .and_then(|mek| {
            if let Some(aad) = aad_owned {
                if let Ok(bytes) = mek.decrypt_with_aad(&message.ciphertext, aad) {
                    return Some(bytes);
                }
            }
            mek.decrypt(&message.ciphertext).ok()
        })
        .and_then(|bytes| String::from_utf8(bytes).ok());
    drop(mek_cache);
    if decrypted.is_some() {
        return decrypted;
    }

    let keystore: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let guard = keystore.lock();
    let ks = guard.as_ref()?;
    if let Some(mek) = crate::keystore::load_channel_mek_generation(
        ks,
        community_id,
        channel_id,
        message.mek_generation,
    ) {
        let plaintext = mek
            .decrypt(&message.ciphertext)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())?;
        state
            .channel_mek_cache
            .lock()
            .insert((community_id.to_string(), channel_id.to_string()), mek);
        return Some(plaintext);
    }

    crate::keystore::load_mek(ks, community_id)
        .filter(|mek| mek.generation() == message.mek_generation)
        .and_then(|mek| {
            let plaintext = mek
                .decrypt(&message.ciphertext)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())?;
            state.mek_cache.lock().insert(community_id.to_string(), mek);
            Some(plaintext)
        })
}

pub(super) async fn message_exists(pool: &DbPool, owner_key: &str, message_id: &str) -> bool {
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
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mut last_error: Option<String> = None;
    for (_segment_index, record_key_str) in segment_records {
        let record_key = match record_key_str.parse::<veilid_core::RecordKey>() {
            Ok(key) => key,
            Err(e) => {
                last_error = Some(format!("invalid channel record key: {e}"));
                continue;
            }
        };
        let value = match rc.get_dht_value(record_key, subkey_index, true).await {
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
    let can_moderate = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return;
        };
        let Some(gov) = community.governance_state.as_ref() else {
            return;
        };
        let Some(pk_hex) = community.my_pseudonym_key.as_ref() else {
            return;
        };
        let Ok(pk_bytes) = hex::decode(pk_hex) else {
            return;
        };
        let Ok(pk_arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) else {
            return;
        };
        let perms = rekindle_governance::permissions::compute_permissions(
            &rekindle_types::id::PseudonymKey(pk_arr),
            None,
            gov,
            rekindle_utils::timestamp_secs(),
        );
        // Architecture §32 W17 — alert any member with a moderation
        // role, not just those who can hand out timeouts. Spec just says
        // "admins"; in our permission model that's the union of
        // ADMINISTRATOR + the message/community/role/ban moderation
        // capabilities. This matches who would actually act on the alert.
        let mod_mask = rekindle_types::permissions::ADMINISTRATOR
            | rekindle_types::permissions::MANAGE_COMMUNITY
            | rekindle_types::permissions::MANAGE_MESSAGES
            | rekindle_types::permissions::TIMEOUT_MEMBERS
            | rekindle_types::permissions::BAN_MEMBERS;
        perms & mod_mask != 0
    };
    if can_moderate {
        crate::event_dispatch::emit_live(
            app_handle,
            "community-event",
            &crate::channels::CommunityEvent::AutoModAlert {
                community_id: community_id.to_string(),
                channel_id: channel_id.to_string(),
                message_id: message_id.to_string(),
                rule_name: rule_name.to_string(),
            },
        );
    }
}

pub fn queue_message_fetch_retry(state: Arc<AppState>, pending: PendingMessageFetch) {
    tokio::spawn(async move {
        tokio::time::sleep(retry::backoff_duration(pending.attempt)).await;
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
    });
}
