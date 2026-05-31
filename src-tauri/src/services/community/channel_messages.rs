//! Phase 19.i-REDO — thin facade.
//!
//! `send_message` + `forward_message` + retry-loop body live in
//! `rekindle_channel::pipeline`. This module constructs a
//! `ChannelAdapter` per call and:
//! - re-exports `PendingChannelMessage` / `SendChannelMessageResult` /
//!   `SentChannelMessage` so existing callers (commands::messaging,
//!   sync_service retry path) compile unchanged
//! - hosts the retry-loop spawn (`start_write_retry_worker`) because
//!   it owns the rx end of `state.channel_write_retry_tx`
//! - hosts `emit_local_chat_event` so the existing src-tauri
//!   commands continue to drive the local UI echo
//! - delegates `enforce_slowmode` + `resolve_outbound_mentions` to
//!   crate primitives via the adapter (files_adapter still calls these)

use std::sync::Arc;

use rekindle_protocol::dht::community::channel_record::ChannelMessage;
use rekindle_records::retry::{self, PendingWrite};
use tauri::Manager;

use crate::channels::{ChatEvent, CommunityEvent};
use crate::db::DbPool;
use crate::state::{AppState, SharedState};
use crate::state_helpers;

/// Pending channel message payload stored in the generic retry queue.
/// Kept as a src-tauri-side struct because sync_service::retry_pending
/// deserialises this exact shape from the pending_messages table.
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
    #[serde(default)]
    pub mentioned_pseudonyms: Vec<String>,
    #[serde(default)]
    pub mentioned_roles: Vec<String>,
    #[serde(default)]
    pub mention_flag_bits: u32,
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

fn build_adapter(
    state: &SharedState,
) -> Result<crate::services::channel_adapter::ChannelAdapter, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    Ok(crate::services::channel_adapter::ChannelAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    ))
}

fn map_result(result: rekindle_channel::ChannelSendResult) -> SentChannelMessage {
    SentChannelMessage {
        result: SendChannelMessageResult {
            status: result.status,
            message_id: result.message_id,
        },
        sender_key: result.sender_pseudonym,
        timestamp: result.timestamp_ms,
        body: result.body,
    }
}

pub async fn send_message(
    state: &SharedState,
    _pool: &DbPool,
    channel_id: &str,
    body: &str,
) -> Result<SentChannelMessage, String> {
    let adapter = build_adapter(state)?;
    // Find the community by walking communities for one that owns this channel.
    let community_id = {
        let communities = state.communities.read();
        communities
            .iter()
            .find_map(|(id, community)| {
                if community.channels.iter().any(|ch| ch.id == channel_id) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| "channel not found in any community".to_string())?
    };
    let result = rekindle_channel::send_channel_message(&adapter, &community_id, channel_id, body)
        .await
        .map_err(|e| e.to_string())?;
    Ok(map_result(result))
}

pub async fn forward_message(
    state: &SharedState,
    _pool: &DbPool,
    source_community_id: &str,
    source_channel_id: &str,
    source_message_id: &str,
    dest_community_id: &str,
    dest_channel_id: &str,
) -> Result<SentChannelMessage, String> {
    let adapter = build_adapter(state)?;
    let result = rekindle_channel::forward_channel_message(
        &adapter,
        source_community_id,
        source_channel_id,
        source_message_id,
        dest_community_id,
        dest_channel_id,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(map_result(result))
}

/// Architecture §28.7 slowmode gate, retained as a re-export for
/// files_adapter callers. Delegates to the crate's
/// `enforce_slowmode_with_bypass`.
pub(crate) fn enforce_slowmode(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::enforce_slowmode_with_bypass(&adapter, community_id, channel_id, now_ms)
        .map_err(|e| e.to_string())
}

/// Sender-side mention resolver, retained as a re-export for
/// files_adapter callers.
pub(crate) fn resolve_outbound_mentions(
    state: &SharedState,
    community_id: &str,
    sender_pseudonym_hex: &str,
    body: &str,
) -> (Vec<String>, Vec<String>, u32) {
    let Ok(adapter) = build_adapter(state) else {
        return (Vec::new(), Vec::new(), 0);
    };
    rekindle_channel::resolve_outbound_mentions(&adapter, community_id, sender_pseudonym_hex, body)
}

/// Spawn the channel-write retry loop. Reads PendingWrite from the
/// mpsc rx end of `state.channel_write_retry_tx` and delegates each
/// attempt to the crate's `process_retry_write` (with backoff +
/// finalize on permanent failure handled here in src-tauri because
/// `find_context_by_record_key` is an AppState scan).
pub fn start_write_retry_worker(state: Arc<AppState>) {
    let (handle, mut rx) = retry::create_write_queue(128);
    *state.channel_write_retry_tx.write() = Some(handle);

    tauri::async_runtime::spawn(async move {
        while let Some(pending) = rx.recv().await {
            process_retry_write(&state, pending).await;
        }
    });
}

async fn process_retry_write(state: &Arc<AppState>, pending: PendingWrite) {
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

async fn retry_write_once(state: &Arc<AppState>, pending: &PendingWrite) -> Result<(), String> {
    let context = find_context_by_record_key(state, &pending.record_key)
        .ok_or_else(|| "channel write retry context not found".to_string())?;
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    let adapter =
        crate::services::channel_adapter::ChannelAdapter::new(Arc::clone(state), app_handle, pool);
    let pending_channel_write = rekindle_channel::deps::PendingChannelWrite {
        record_key: pending.record_key.clone(),
        subkey: pending.subkey,
        data: pending.data.clone(),
    };
    rekindle_channel::process_retry_write(&adapter, &pending_channel_write, &context)
        .await
        .map_err(|e| e.to_string())
}

fn find_context_by_record_key(
    state: &SharedState,
    record_key: &str,
) -> Option<rekindle_channel::deps::ChannelWriteContext> {
    let communities = state.communities.read();
    for (community_id, community) in &*communities {
        let slot_keypair_str = community.slot_keypair.clone()?;
        let slot_index = community.my_subkey_index?;
        let segment_index = community.my_segment_index.unwrap_or(0);
        for (channel_id, channel_key) in &community.channel_log_keys {
            if channel_key == record_key {
                return Some(rekindle_channel::deps::ChannelWriteContext {
                    community_id: community_id.clone(),
                    channel_id: channel_id.clone(),
                    channel_key: channel_key.clone(),
                    slot_keypair_str,
                    slot_index,
                    segment_index,
                });
            }
        }
    }
    None
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
    let Some(app_handle) = state_helpers::app_handle(state) else {
        return;
    };
    let event = CommunityEvent::ChannelMessageDeliveryFailed {
        community_id: context.community_id,
        channel_id: context.channel_id,
        message_id,
    };
    crate::event_dispatch::emit_live(&app_handle, "community-event", &event);
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
    crate::event_dispatch::emit_live(app, "chat-event", &event);
}

