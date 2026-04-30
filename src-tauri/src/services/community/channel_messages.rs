use std::sync::Arc;

use rekindle_protocol::dht::community::channel_record::ChannelMessage;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_records::retry::{self, PendingWrite};
use tauri::Emitter;

use crate::channels::{ChatEvent, CommunityEvent};
use crate::db::DbPool;
use crate::db_helpers::{db_call, db_fire};
use crate::state::{AppState, SharedState};
use crate::state_helpers;

/// Pending channel message payload stored in the generic retry queue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChannelMessage {
    pub community_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub author_pseudonym: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: i64,
    pub subkey_index: u32,
    pub lamport_ts: u64,
    pub sequence: u64,
    pub content_hash: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageResult {
    pub status: String,
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct SentChannelMessage {
    pub result: SendChannelMessageResult,
    pub sender_key: String,
    pub timestamp: u64,
    pub body: String,
}

struct ChannelWriteContext {
    community_id: String,
    channel_id: String,
    channel_key: String,
    slot_keypair: String,
    slot_index: u32,
}

pub fn start_write_retry_worker(state: Arc<AppState>) {
    let (handle, mut rx) = retry::create_write_queue(128);
    *state.channel_write_retry_tx.write() = Some(handle);

    tauri::async_runtime::spawn(async move {
        while let Some(pending) = rx.recv().await {
            process_retry_write(&state, pending).await;
        }
    });
}

pub async fn send_message(
    state: &SharedState,
    pool: &DbPool,
    channel_id: &str,
    body: &str,
) -> Result<SentChannelMessage, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let timestamp_ms = crate::db::timestamp_now();

    let (community_id, mek_generation) = {
        let communities = state.communities.read();
        let community = communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .ok_or("channel not found in any community")?;
        (community.id.clone(), community.mek_generation)
    };

    crate::commands::community::require_permission(
        state,
        &community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::SEND_MESSAGES,
    )?;

    let sender_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_else(|| owner_key.clone())
    };

    let ciphertext = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(&community_id).ok_or_else(|| {
            "MEK not available — rejoin the community or wait for MEK delivery".to_string()
        })?;
        mek.encrypt(body.as_bytes())
            .map_err(|e| format!("MEK encryption failed: {e}"))?
    };

    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let lamport_ts = state_helpers::increment_lamport(state, &community_id);
    let sequence = {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&community_id) {
            let s = cs
                .channel_sequences
                .entry(channel_id.to_string())
                .or_insert(0);
            *s += 1;
            *s
        } else {
            1
        }
    };

    let channel_id_owned = channel_id.to_string();
    let sender_key_owned = sender_key.clone();
    let body_owned = body.to_string();
    let owner_key_owned = owner_key.clone();
    let message_id_for_db = message_id.clone();
    db_call(pool, move |conn| {
        crate::message_repo::insert_channel_message_with_protocol_metadata(
            conn,
            &owner_key_owned,
            &channel_id_owned,
            &sender_key_owned,
            &body_owned,
            timestamp_ms,
            true,
            Some(i64::try_from(mek_generation).unwrap_or(i64::MAX)),
            &message_id_for_db,
            lamport_ts,
            false,
        )
    })
    .await?;

    let context = channel_write_context(state, &community_id, channel_id)?;
    persist_channel_sequence(pool, &owner_key, &community_id, channel_id, sequence);

    let channel_msg = ChannelMessage {
        sequence,
        sender_pseudonym: sender_key.clone(),
        ciphertext: ciphertext.clone(),
        mek_generation,
        timestamp: u64::try_from(timestamp_ms).unwrap_or_default(),
        reply_to: None,
        lamport_ts,
        message_id: Some(message_id.clone()),
    };

    let status = match write_message_once(state, &context, &channel_msg, context.slot_index).await {
        Ok(()) => "delivered".to_string(),
        Err(error) => {
            tracing::warn!(error = %error, "channel delivery failed; queueing retry");
            enqueue_retry(
                state,
                &context.channel_key,
                crate::services::community::channel_message_subkey(context.slot_index),
                &channel_msg,
            )
            .await?;
            "queued".to_string()
        }
    };

    Ok(SentChannelMessage {
        result: SendChannelMessageResult { status, message_id },
        sender_key,
        timestamp: u64::try_from(timestamp_ms).unwrap_or_default(),
        body: body.to_string(),
    })
}

async fn enqueue_retry(
    state: &SharedState,
    record_key: &str,
    subkey: u32,
    message: &ChannelMessage,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(message).map_err(|e| format!("serialize retry write: {e}"))?;
    let handle = state
        .channel_write_retry_tx
        .read()
        .clone()
        .ok_or("channel write retry queue unavailable")?;
    handle.enqueue(record_key.to_string(), subkey, bytes).await;
    Ok(())
}

async fn process_retry_write(state: &SharedState, pending: PendingWrite) {
    let mut current = pending;
    loop {
        match retry_write_once(state, &current).await {
            Ok(()) => return,
            Err(error) => {
                tracing::debug!(
                    record_key = %current.record_key,
                    subkey = current.subkey,
                    attempt = current.attempt,
                    error = %error,
                    "channel write retry attempt failed"
                );
                if let Some(next) = retry::retry_or_fail(&current) {
                    tokio::time::sleep(retry::backoff_duration(current.attempt)).await;
                    current = next;
                    continue;
                }
                emit_delivery_failed(state, &current, &error);
                return;
            }
        }
    }
}

async fn retry_write_once(state: &SharedState, pending: &PendingWrite) -> Result<(), String> {
    let context = find_context_by_record_key(state, &pending.record_key)
        .ok_or("channel write retry context not found")?;
    let message: ChannelMessage = serde_json::from_slice(&pending.data)
        .map_err(|e| format!("deserialize queued channel message: {e}"))?;
    write_message_once(state, &context, &message, context.slot_index).await?;
    emit_delivery_event(
        state,
        CommunityEvent::ChannelMessageDelivered {
            community_id: context.community_id,
            channel_id: context.channel_id,
            message_id: message
                .message_id
                .ok_or("queued channel message missing message_id")?,
        },
    );
    Ok(())
}

async fn write_message_once(
    state: &SharedState,
    context: &ChannelWriteContext,
    channel_msg: &ChannelMessage,
    slot_index: u32,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let writer = context
        .slot_keypair
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    rekindle_protocol::dht::community::channel_record::write_member_message(
        &mgr,
        &context.channel_key,
        slot_index,
        writer,
        channel_msg,
    )
    .await
    .map_err(|e| format!("SMPL channel write failed: {e}"))?;

    let notification = build_message_notification(&context.channel_id, channel_msg, slot_index)?;
    crate::services::community::send_to_mesh(state, &context.community_id, &notification)?;
    Ok(())
}

fn build_message_notification(
    channel_id: &str,
    channel_msg: &ChannelMessage,
    slot_index: u32,
) -> Result<CommunityEnvelope, String> {
    Ok(CommunityEnvelope::MessageNotification {
        channel_id: channel_id.to_string(),
        message_id: channel_msg
            .message_id
            .clone()
            .ok_or("channel message missing message_id")?,
        author_pseudonym: channel_msg.sender_pseudonym.clone(),
        subkey_index: crate::services::community::channel_message_subkey(slot_index),
        lamport_ts: channel_msg.lamport_ts,
        sequence: channel_msg.sequence,
        content_hash: blake3::hash(&channel_msg.ciphertext).to_hex().to_string(),
        timestamp: channel_msg.timestamp,
    })
}

fn channel_write_context(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<ChannelWriteContext, String> {
    let communities = state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or("community not found for channel send")?;
    Ok(ChannelWriteContext {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        channel_key: community
            .channel_log_keys
            .get(channel_id)
            .cloned()
            .ok_or("channel record key missing")?,
        slot_keypair: community
            .slot_keypair
            .clone()
            .ok_or("slot keypair missing")?,
        slot_index: community
            .my_subkey_index
            .ok_or("community subkey index missing")?,
    })
}

fn find_context_by_record_key(
    state: &SharedState,
    record_key: &str,
) -> Option<ChannelWriteContext> {
    let communities = state.communities.read();
    for (community_id, community) in &*communities {
        let slot_keypair = community.slot_keypair.clone()?;
        let slot_index = community.my_subkey_index?;
        for (channel_id, channel_key) in &community.channel_log_keys {
            if channel_key == record_key {
                return Some(ChannelWriteContext {
                    community_id: community_id.clone(),
                    channel_id: channel_id.clone(),
                    channel_key: channel_key.clone(),
                    slot_keypair,
                    slot_index,
                });
            }
        }
    }
    None
}

fn persist_channel_sequence(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    sequence: u64,
) {
    let owner_key = owner_key.to_string();
    let community_id = community_id.to_string();
    let channel_id = channel_id.to_string();
    let sequence = i64::try_from(sequence).unwrap_or(i64::MAX);
    db_fire(pool, "persist channel sequence", move |conn| {
        conn.execute(
            "UPDATE channels SET my_sequence = ?1 WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
            rusqlite::params![sequence, owner_key, community_id, channel_id],
        )?;
        Ok(())
    });
}

fn emit_delivery_failed(state: &SharedState, pending: &PendingWrite, error: &str) {
    let Some(context) = find_context_by_record_key(state, &pending.record_key) else {
        return;
    };
    let Ok(message) = serde_json::from_slice::<ChannelMessage>(&pending.data) else {
        return;
    };
    let Some(message_id) = message.message_id else {
        return;
    };
    tracing::warn!(
        community = %context.community_id,
        channel = %context.channel_id,
        message_id = %message_id,
        error,
        "queued channel write permanently failed"
    );
    emit_delivery_event(
        state,
        CommunityEvent::ChannelMessageDeliveryFailed {
            community_id: context.community_id,
            channel_id: context.channel_id,
            message_id,
        },
    );
}

fn emit_delivery_event(state: &SharedState, event: CommunityEvent) {
    let Some(app_handle) = state_helpers::app_handle(state) else {
        return;
    };
    let _ = app_handle.emit("community-event", event);
}

pub fn emit_local_chat_event(app: &tauri::AppHandle, sent: &SentChannelMessage, channel_id: &str) {
    let event = ChatEvent::MessageReceived {
        from: sent.sender_key.clone(),
        body: sent.body.clone(),
        decryption_failed: false,
        automod_blurred: false,
        timestamp: sent.timestamp,
        conversation_id: channel_id.to_string(),
        server_message_id: Some(sent.result.message_id.clone()),
        reply_to_id: None,
        sender_display_name: None,
    };
    let _ = app.emit("chat-event", &event);
}

#[cfg(test)]
#[path = "channel_messages_tests.rs"]
mod tests;
