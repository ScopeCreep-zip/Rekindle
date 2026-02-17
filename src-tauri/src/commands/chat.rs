use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::channels::ChatEvent;
use crate::commands::auth::current_owner_key;
use crate::db::{self, DbPool};
use crate::services;
use crate::state::SharedState;

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
    let owner_key = current_owner_key(state.inner())?;
    let sender_key = owner_key.clone();
    let timestamp = db::timestamp_now();

    tracing::info!(to = %to, from = %sender_key, len = body.len(), "sending message");

    // Step 1: Persist to SQLite FIRST (before network send)
    let pool_clone = pool.inner().clone();
    let to_clone = to.clone();
    let sender_key_clone = sender_key.clone();
    let body_clone = body.clone();
    let ok = owner_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read) \
             VALUES (?, ?, 'dm', ?, ?, ?, 1)",
            rusqlite::params![ok, to_clone, sender_key_clone, body_clone, timestamp],
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Step 2: Send via Veilid (best-effort — queues on failure internally)
    if let Err(e) = services::message_service::send_message(state.inner(), pool.inner(), &to, &body).await {
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
    state
        .identity
        .read()
        .as_ref()
        .ok_or("not logged in")?;

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
    let our_key = current_owner_key(state.inner()).unwrap_or_default();

    let pool = pool.inner().clone();
    let peer_id_clone = peer_id.clone();
    let ok = our_key.clone();
    let messages = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, sender_key, body, timestamp FROM messages \
                 WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'dm' \
                 ORDER BY timestamp ASC LIMIT ?",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(rusqlite::params![ok, peer_id_clone, limit], |row| {
                let sender = db::get_str(row, "sender_key");
                let is_own = sender == our_key;
                Ok(Message {
                    id: db::get_i64(row, "id"),
                    sender_id: sender,
                    body: db::get_str(row, "body"),
                    timestamp: db::get_i64(row, "timestamp"),
                    is_own,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row.map_err(|e| e.to_string())?);
        }
        Ok::<_, String>(messages)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(messages)
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
    let dht_record_key = {
        let friends = state.friends.read();
        friends.get(&peer_id).and_then(|f| f.dht_record_key.clone())
    };
    let Some(dht_key_str) = dht_record_key else {
        return Ok(()); // No DHT key — nothing to sync
    };
    let record_key: veilid_core::RecordKey = dht_key_str
        .parse()
        .map_err(|e| format!("invalid DHT key: {e}"))?;

    let (routing_context, api) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        if !nh.is_attached {
            return Ok(());
        }
        (nh.routing_context.clone(), nh.api.clone())
    };

    // Open (no-op if already open) and force-refresh route blob (subkey 6)
    let _ = routing_context
        .open_dht_record(record_key.clone(), None)
        .await;
    if let Ok(Some(value_data)) = routing_context
        .get_dht_value(record_key, 6, true)
        .await
    {
        let route_blob = value_data.data().to_vec();
        if !route_blob.is_empty() {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager.cache_route(&api, &peer_id, route_blob);
            }
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
    let owner_key = current_owner_key(state.inner())?;

    if let Some(friend) = state.friends.write().get_mut(&peer_id) {
        friend.unread_count = 0;
    }

    let pool = pool.inner().clone();
    let peer_id_clone = peer_id.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE messages SET is_read = 1 WHERE owner_key = ? AND conversation_id = ? AND is_read = 0",
            rusqlite::params![owner_key, peer_id_clone],
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(())
}
