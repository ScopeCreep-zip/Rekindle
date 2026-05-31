//! Phase 23.C — chat-handler Tauri-runtime orchestration lifted from
//! `commands/chat.rs`. Same pattern as `friend_runtime`: the
//! `#[tauri::command]` handler in `commands/` stays a ≤15-LoC
//! delegation; persistence + transport + state mutation lives here.

use std::sync::Arc;

use crate::channels::ChatEvent;
use crate::commands::chat::Message;
use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::services;
use crate::state::AppState;
use crate::state_helpers;

pub async fn send_dm_inner(
    state: Arc<AppState>,
    pool: DbPool,
    app: tauri::AppHandle,
    to: String,
    body: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;
    let sender_key = owner_key.clone();
    let timestamp = db::timestamp_now();

    tracing::info!(to = %to, from = %sender_key, len = body.len(), "sending message");

    let to_clone = to.clone();
    let sender_key_clone = sender_key.clone();
    let body_clone = body.clone();
    let ok = owner_key.clone();
    db_call(&pool, move |conn| {
        crate::message_repo::insert_dm(
            conn,
            &ok,
            &to_clone,
            &sender_key_clone,
            &body_clone,
            timestamp,
            true,
        )
    })
    .await?;

    if let Err(e) = services::message_service::send_message(&state, &pool, &to, &body).await {
        tracing::warn!(error = %e, "DM send failed — message persisted locally");
    }

    let ack = ChatEvent::MessageAck {
        message_id: timestamp.cast_unsigned(),
    };
    crate::event_dispatch::emit_live(&app, "chat-event", &ack);

    Ok(())
}

pub async fn get_message_history_inner(
    state: Arc<AppState>,
    pool: DbPool,
    peer_id: String,
    limit: u32,
) -> Result<Vec<Message>, String> {
    let our_key = state_helpers::owner_key_or_default(&state);

    let ok = our_key.clone();
    db_call(&pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, timestamp FROM messages \
                 WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'dm' \
                 ORDER BY timestamp ASC LIMIT ?",
        )?;

        let rows = stmt.query_map(rusqlite::params![ok, peer_id, limit], |row| {
            let sender = db::get_str(row, "sender_key");
            let is_own = sender == our_key;
            Ok(Message {
                id: db::get_i64(row, "id"),
                sender_id: sender,
                body: db::get_str(row, "body"),
                decryption_failed: false,
                automod_blurred: false,
                timestamp: db::get_i64(row, "timestamp"),
                is_own,
                server_message_id: None,
                reactions: None,
                pinned: None,
                poll: None,
                forwarded_from_author: None,
                attachment: None,
                flags: 0,
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    })
    .await
}

pub async fn prepare_chat_session_inner(
    state: Arc<AppState>,
    peer_id: String,
) -> Result<(), String> {
    let dht_record_key = state_helpers::friend_dht_key(&state, &peer_id);
    let Some(dht_key_str) = dht_record_key else {
        return Ok(());
    };
    let record_key: veilid_core::RecordKey = dht_key_str
        .parse()
        .map_err(|e| format!("invalid DHT key: {e}"))?;

    let Some((_api, routing_context)) = state_helpers::safe_api_and_routing_context(&state) else {
        return Ok(());
    };

    let _ = routing_context.open_dht_record(record_key.clone(), None).await;
    if let Ok(Some(value_data)) = routing_context.get_dht_value(record_key, 6, true).await {
        let route_blob = value_data.data().to_vec();
        if !route_blob.is_empty() {
            state_helpers::cache_peer_route(&state, &peer_id, route_blob);
        }
    }

    tracing::debug!(peer = %peer_id, "prepared chat session — route refreshed from DHT");
    Ok(())
}

pub async fn mark_read_inner(
    state: Arc<AppState>,
    pool: DbPool,
    peer_id: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;

    if let Some(friend) = state.friends.write().get_mut(&peer_id) {
        friend.unread_count = 0;
    }

    let peer_id_clone = peer_id.clone();
    db_call(&pool, move |conn| {
        conn.execute(
            "UPDATE messages SET is_read = 1 WHERE owner_key = ? AND conversation_id = ? AND is_read = 0",
            rusqlite::params![owner_key, peer_id_clone],
        )?;
        Ok(())
    })
    .await?;

    Ok(())
}
