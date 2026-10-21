use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::message::SendState;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::host_call;
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
    /// Our own message's delivery state (`queued`, `sent`, `failed`, …);
    /// `delivered` for a received one (plan D1, C7.14).
    pub send_state: SendState,
}

/// The `send_state` column of a `messages` row (the schema's CHECK keeps it
/// in the vocabulary).
///
/// # Errors
/// The column is missing or holds an unknown state.
pub fn stored_send_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<SendState> {
    let stored: String = row.get("send_state")?;
    SendState::parse(&stored).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown send state {stored}").into(),
        )
    })
}

/// The state of a message read back from the network: at consensus, so
/// `sent` when it is ours and `delivered` otherwise.
#[must_use]
pub fn network_send_state(is_own: bool) -> SendState {
    if is_own {
        SendState::Sent
    } else {
        SendState::Delivered
    }
}

/// An attachment as the webview sees it. The on-disk path of a downloaded
/// file stays in the backend (it carries the OS user name); the webview
/// only learns that the file was saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAttachmentDto {
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub total_size: u64,
    pub chunk_count: u32,
    pub downloaded: bool,
}

impl MessageAttachmentDto {
    /// Build from a parsed `messages.attachment_json` value.
    pub fn from_record(record: rekindle_files::AttachmentRecordJson) -> Self {
        Self {
            downloaded: record.local_path.is_some(),
            attachment_id: record.attachment_id,
            filename: record.filename,
            mime_type: record.mime_type,
            total_size: record.total_size,
            chunk_count: record.chunk_count,
        }
    }
}

/// Send a direct message (`IpcRequest::DmSend`), a reply to `reply_to`
/// (the parent's client id) when given; the stored message.
#[tauri::command]
pub async fn send_message(
    to: String,
    body: String,
    reply_to: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<DmMessage>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::DmSend {
            peer_key: to,
            body,
            reply_to,
        },
    )
    .await
}

/// Send our failed message again by its client id (`IpcRequest::DmResend`);
/// the stored row goes out under the same id, so no new row appears.
#[tauri::command]
pub async fn resend_message(
    message_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<DmMessage>, CommandError> {
    host_call::call(&state, IpcRequest::DmResend { message_id }).await
}

/// Report the draft to a peer (`IpcRequest::DmTyping`): `typing` is whether
/// it holds text. The Host sends the start, its refresh and the idle stop
/// (plan E2.3); typing is dropped, never queued, when there is no route.
#[tauri::command]
pub async fn send_typing(
    peer_id: String,
    typing: bool,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::DmTyping {
            peer_key: peer_id,
            typing,
        },
    )
    .await
}

/// The newest `limit` messages with a friend before `before` (ms), oldest
/// first (`IpcRequest::DmHistory`).
#[tauri::command]
pub async fn get_message_history(
    peer_id: String,
    limit: u32,
    before: Option<i64>,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<DmMessage>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::DmHistory {
            peer_key: peer_id,
            limit,
            before,
        },
    )
    .await
}

/// Mark everything from a friend read (`IpcRequest::DmMarkRead`), with the
/// client id of the newest of their messages the focused window rendered.
#[tauri::command]
pub async fn mark_read(
    peer_id: String,
    rendered_through: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    let request = IpcRequest::DmMarkRead {
        peer_key: peer_id,
        rendered_through,
    };
    host_call::call(&state, request).await
}
