use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::channels::ChatEvent;
use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::services;
use crate::state::SharedState;
use crate::state_helpers;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: i64,
    pub sender_id: String,
    pub body: String,
    pub timestamp: i64,
    pub is_own: bool,
}

/// Send a message to a friend (1:1 DM).
#[tauri::command]
pub async fn send_message(
    to: String,
    body: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let sender_key = owner_key.clone();
    let timestamp = db::timestamp_now();

    tracing::info!(to = %to, from = %sender_key, len = body.len(), "sending message");

    // Step 1: Persist to SQLite FIRST (before network send)
    let to_clone = to.clone();
    let sender_key_clone = sender_key.clone();
    let body_clone = body.clone();
    let ok = owner_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read) \
             VALUES (?, ?, 'dm', ?, ?, ?, 1)",
            rusqlite::params![ok, to_clone, sender_key_clone, body_clone, timestamp],
        )?;
        Ok(())
    })
    .await?;

    // Step 2: Send via Veilid (best-effort — queues on failure internally)
    if let Err(e) =
        services::message_service::send_message(state.inner(), pool.inner(), &to, &body).await
    {
        tracing::warn!(error = %e, "DM send failed — message persisted locally");
    }

    // Step 3: Emit ack
    let ack = ChatEvent::MessageAck {
        message_id: timestamp.cast_unsigned(),
    };
    let _ = app.emit("chat-event", &ack);

    Ok(())
}

/// Send a typing indicator to a peer.
///
/// Uses a structured `TypingIndicator` payload wrapped in a signed `MessageEnvelope`.
/// Typing indicators are ephemeral — if no route exists, they are silently dropped.
#[tauri::command]
pub async fn send_typing(
    peer_id: String,
    typing: bool,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Verify identity is loaded
    state.identity.read().as_ref().ok_or("not logged in")?;

    services::message_service::send_typing(state.inner(), pool.inner(), &peer_id, typing).await
}

/// Get chat history from `SQLite`.
#[tauri::command]
pub async fn get_message_history(
    peer_id: String,
    limit: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    let our_key = state_helpers::owner_key_or_default(state.inner());

    let ok = our_key.clone();
    db_call(pool.inner(), move |conn| {
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
                timestamp: db::get_i64(row, "timestamp"),
                is_own,
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

/// Proactively refresh a peer's route before sending messages.
///
/// Called when a chat window opens to ensure the cached route is fresh,
/// reducing first-message delivery failures from stale routes.
#[tauri::command]
pub async fn prepare_chat_session(
    peer_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let dht_record_key = state_helpers::friend_dht_key(state.inner(), &peer_id);
    let Some(dht_key_str) = dht_record_key else {
        return Ok(()); // No DHT key — nothing to sync
    };
    let record_key: veilid_core::RecordKey = dht_key_str
        .parse()
        .map_err(|e| format!("invalid DHT key: {e}"))?;

    let Some((_api, routing_context)) = state_helpers::api_and_routing_context(state.inner())
    else {
        return Ok(());
    };

    // Open (no-op if already open) and force-refresh route blob (subkey 6)
    let _ = routing_context
        .open_dht_record(record_key.clone(), None)
        .await;
    if let Ok(Some(value_data)) = routing_context.get_dht_value(record_key, 6, true).await {
        let route_blob = value_data.data().to_vec();
        if !route_blob.is_empty() {
            state_helpers::cache_peer_route(state.inner(), &peer_id, route_blob);
        }
    }

    tracing::debug!(peer = %peer_id, "prepared chat session — route refreshed from DHT");
    Ok(())
}

/// Mark messages as read.
#[tauri::command]
pub async fn mark_read(
    peer_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    if let Some(friend) = state.friends.write().get_mut(&peer_id) {
        friend.unread_count = 0;
    }

    let peer_id_clone = peer_id.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "UPDATE messages SET is_read = 1 WHERE owner_key = ? AND conversation_id = ? AND is_read = 0",
            rusqlite::params![owner_key, peer_id_clone],
        )?;
        Ok(())
    })
    .await?;

    Ok(())
}
