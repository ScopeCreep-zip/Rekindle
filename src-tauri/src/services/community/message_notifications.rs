use std::sync::Arc;

use rekindle_protocol::dht::community::channel_record::{
    decode_channel_entries, ChannelMessage, ChannelRecordEntry, CHANNEL_OWNER_SUBKEY_COUNT,
};
use rekindle_records::retry;
use tauri::{Emitter, Manager};

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::{db_call, db_fire};
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

pub fn channel_message_subkey(member_index: u32) -> u32 {
    u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + member_index
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn verify_notification_message(
    pending: &PendingMessageFetch,
    message: &ChannelMessage,
) -> Result<(), &'static str> {
    if blake3_hex(&message.ciphertext) != pending.content_hash {
        return Err("message notification hash mismatch");
    }
    Ok(())
}

fn emit_message_received(
    app_handle: &tauri::AppHandle,
    pending: &PendingMessageFetch,
    from: String,
    body: String,
    timestamp: u64,
    decryption_failed: bool,
    automod_blurred: bool,
) {
    let _ = app_handle.emit(
        "chat-event",
        &ChatEvent::MessageReceived {
            from,
            body,
            decryption_failed,
            automod_blurred,
            timestamp,
            conversation_id: pending.channel_id.clone(),
            server_message_id: Some(pending.message_id.clone()),
            reply_to_id: None,
            sender_display_name: None,
        },
    );
}

fn decrypt_message_body(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    message: &ChannelMessage,
) -> Option<String> {
    {
        let channel_mek_cache = state.channel_mek_cache.lock();
        if let Some(mek) =
            channel_mek_cache.get(&(community_id.to_string(), channel_id.to_string()))
        {
            if mek.generation() == message.mek_generation {
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
        .and_then(|mek| mek.decrypt(&message.ciphertext).ok())
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

async fn message_exists(pool: &DbPool, owner_key: &str, message_id: &str) -> bool {
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

async fn fetch_channel_message(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    subkey_index: u32,
    message_id: &str,
) -> Result<ChannelMessage, String> {
    let channel_record_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|community| community.channel_log_keys.get(channel_id).cloned())
            .ok_or("channel record key not found")?
    };
    let record_key = channel_record_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid channel record key: {e}"))?;
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let value = rc
        .get_dht_value(record_key, subkey_index, true)
        .await
        .map_err(|e| format!("get_dht_value failed: {e}"))?
        .ok_or("channel subkey is empty")?;
    let entries = decode_channel_entries(value.data())
        .map_err(|e| format!("invalid channel page payload: {e}"))?;
    entries
        .into_iter()
        .find_map(|entry| match entry {
            ChannelRecordEntry::Message(message)
                if message.message_id.as_deref() == Some(message_id) =>
            {
                Some(message)
            }
            _ => None,
        })
        .ok_or("message id not found in channel page".into())
}

fn update_peer_sequence(
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

fn emit_automod_alert(
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
        perms & rekindle_types::permissions::TIMEOUT_MEMBERS
            == rekindle_types::permissions::TIMEOUT_MEMBERS
            || perms & rekindle_types::permissions::ADMINISTRATOR
                == rekindle_types::permissions::ADMINISTRATOR
    };
    if can_moderate {
        let _ = app_handle.emit(
            "community-event",
            crate::channels::CommunityEvent::AutoModAlert {
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
            let _ = handle_message_notification(
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

pub async fn handle_message_notification(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pending: PendingMessageFetch,
) -> Result<(), String> {
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("owner key unavailable".into());
    }
    if message_exists(pool.inner(), &owner_key, &pending.message_id).await {
        return Ok(());
    }

    let message = match fetch_channel_message(
        state,
        &pending.community_id,
        &pending.channel_id,
        pending.subkey_index,
        &pending.message_id,
    )
    .await
    {
        Ok(message) => message,
        Err(error) => {
            if pending.attempt + 1 < retry::MAX_RETRIES {
                queue_message_fetch_retry(state.clone(), pending);
            }
            return Err(error);
        }
    };

    if let Err(error) = verify_notification_message(&pending, &message) {
        if pending.attempt + 1 < retry::MAX_RETRIES {
            queue_message_fetch_retry(state.clone(), pending.clone());
        }
        return Err(error.into());
    }

    state_helpers::merge_lamport(state, &pending.community_id, message.lamport_ts);
    let Some(body) = decrypt_message_body(
        app_handle,
        state,
        &pending.community_id,
        &pending.channel_id,
        &message,
    ) else {
        let requester_pseudonym = {
            let communities = state.communities.read();
            communities
                .get(&pending.community_id)
                .and_then(|community| community.my_pseudonym_key.clone())
        };
        if let Some(requester_pseudonym) = requester_pseudonym {
            let request = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                rekindle_protocol::dht::community::envelope::ControlPayload::RequestMEK {
                    channel_id: pending.channel_id.clone(),
                    needed_generation: message.mek_generation,
                    requester_pseudonym,
                },
            );
            let _ =
                crate::services::community::send_to_mesh(state, &pending.community_id, &request);
        }
        emit_message_received(
            app_handle,
            &pending,
            message.sender_pseudonym.clone(),
            String::new(),
            message.timestamp,
            true,
            false,
        );
        update_peer_sequence(
            state,
            &pending.community_id,
            &message.sender_pseudonym,
            &pending.channel_id,
            pending.sequence,
        );
        return Ok(());
    };

    let automod_action =
        crate::services::community::automod::evaluate_message(state, &pending.community_id, &body)
            .unwrap_or(crate::services::community::automod::AutoModAction::Allow);
    if automod_action == crate::services::community::automod::AutoModAction::BlockLocally {
        tracing::info!(
            community_id = %pending.community_id,
            channel_id = %pending.channel_id,
            message_id = %pending.message_id,
            "automod blocked"
        );
        update_peer_sequence(
            state,
            &pending.community_id,
            &message.sender_pseudonym,
            &pending.channel_id,
            pending.sequence,
        );
        return Ok(());
    }

    let message_id = pending.message_id.clone();
    let channel_id = pending.channel_id.clone();
    let sender = message.sender_pseudonym.clone();
    let body_for_db = body.clone();
    let timestamp = i64::try_from(message.timestamp).unwrap_or(i64::MAX);
    let mek_generation = i64::try_from(message.mek_generation).unwrap_or(i64::MAX);
    let lamport_ts = message.lamport_ts;
    let automod_blurred =
        automod_action == crate::services::community::automod::AutoModAction::BlurContent;
    db_fire(
        pool.inner(),
        "store notified channel message",
        move |conn| {
            crate::message_repo::insert_channel_message_with_protocol_metadata(
                conn,
                &owner_key,
                &channel_id,
                &sender,
                &body_for_db,
                timestamp,
                false,
                Some(mek_generation),
                &message_id,
                lamport_ts,
                automod_blurred,
            )
        },
    );

    update_peer_sequence(
        state,
        &pending.community_id,
        &message.sender_pseudonym,
        &pending.channel_id,
        pending.sequence,
    );

    let sender_pseudonym = message.sender_pseudonym.clone();
    emit_message_received(
        app_handle,
        &pending,
        sender_pseudonym.clone(),
        body.clone(),
        message.timestamp,
        false,
        automod_blurred,
    );

    if automod_action == crate::services::community::automod::AutoModAction::AlertModerators {
        let rule_name = crate::services::community::automod::list_rules(state, &pending.community_id)
            .ok()
            .and_then(|rules| {
                rules
                    .into_iter()
                    .find(|rule| {
                        rule.enabled
                            && rule.action == "alert_moderators"
                            && (rule
                                .keywords
                                .iter()
                                .any(|keyword| body.to_lowercase().contains(&keyword.to_lowercase()))
                                || rule.regex_patterns.iter().any(|pattern| {
                                    regex::Regex::new(pattern)
                                        .map(|compiled| compiled.is_match(&body))
                                        .unwrap_or(false)
                                }))
                    })
                    .map(|rule| rule.name)
            })
            .unwrap_or_else(|| "AutoMod".to_string());
        emit_automod_alert(
            app_handle,
            state,
            &pending.community_id,
            &pending.channel_id,
            &pending.message_id,
            &rule_name,
        );
    }

    if crate::services::community::should_emit_message_notification(
        state,
        pool.inner(),
        &pending.community_id,
        &pending.channel_id,
        &body,
    )
    .await
    .unwrap_or(false)
    {
        crate::services::community::emit_message_notification(
            app_handle,
            state,
            &pending.community_id,
            &pending.channel_id,
            &sender_pseudonym,
            &body,
        );
    }

    Ok(())
}

#[cfg(test)]
#[path = "message_notifications_tests.rs"]
mod tests;
