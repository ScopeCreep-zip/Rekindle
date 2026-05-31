use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::DbPool;
use crate::services;
use crate::state::SharedState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionGroup {
    pub emoji: String,
    pub count: u32,
    pub reactors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePollAnswer {
    pub index: u8,
    pub text: String,
    pub vote_count: u32,
    pub voters: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePoll {
    pub poll_id: String,
    pub question: String,
    pub answers: Vec<MessagePollAnswer>,
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub closed: bool,
    #[serde(default)]
    pub selected_answers: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: i64,
    pub sender_id: String,
    pub body: String,
    #[serde(default)]
    pub decryption_failed: bool,
    #[serde(default)]
    pub automod_blurred: bool,
    pub timestamp: i64,
    pub is_own: bool,
    /// Message ID (present for community channel messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactions: Option<Vec<ReactionGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<MessagePoll>,
    /// Pseudonym (hex) of the original author when this row was a forward.
    /// `None` for native messages. Frontend renders a "Forwarded from X" header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarded_from_author: Option<String>,
    /// Lost Cargo attachment metadata (architecture §28.9). Decoded from
    /// the SQLite `attachment_json` column. `None` for plain messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<MessageAttachmentDto>,
    /// Bitfield from `ChannelEntry::Message.flags` — VOICE_MESSAGE=0x10
    /// (architecture §16.4), SUPPRESS_NOTIFICATIONS=0x20, etc. Frontend
    /// branches on these to switch render mode (voice player vs text).
    #[serde(default)]
    pub flags: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAttachmentDto {
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub total_size: u64,
    pub chunk_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

/// Send a message to a friend (1:1 DM).
///
/// Phase 8 — `idempotency_key` (UUID v7) dedupes click-spam. Frontend
/// generates one key per user gesture; rapid re-clicks within the same
/// gesture reuse it and short-circuit to the cached response.
#[tauri::command]
pub async fn send_message(
    to: String,
    body: String,
    idempotency_key: uuid::Uuid,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    let s = state.inner().clone();
    let p = pool.inner().clone();
    state
        .idempotency
        .wrap(idempotency_key, || async move {
            services::chat_runtime::send_dm_inner(s, p, app, to, body).await
        })
        .await
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
    services::chat_runtime::get_message_history_inner(
        state.inner().clone(),
        pool.inner().clone(),
        peer_id,
        limit,
    )
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
    services::chat_runtime::prepare_chat_session_inner(state.inner().clone(), peer_id).await
}

/// Mark messages as read.
#[tauri::command]
pub async fn mark_read(
    peer_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    services::chat_runtime::mark_read_inner(state.inner().clone(), pool.inner().clone(), peer_id)
        .await
}
