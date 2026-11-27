use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::message::SendState;
use tauri::State;

use crate::state::SharedState;

/// A channel message and its parts, in the Host's DTO shape (plan E3.1 E2c).
pub use rekindle_types::dto::channel_message::{
    ChannelMessageView as Message, MessageAttachment as MessageAttachmentDto, MessagePoll,
    MessagePollAnswer, ReactionGroup,
};

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

host_command! {
    state;
    /// Send a direct message (`IpcRequest::DmSend`), a reply to `reply_to`
    /// (the parent's client id) when given; the stored message.
    pub async fn send_message(
        to: String,
        body: String,
        reply_to: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<DmMessage>, CommandError> {
        IpcRequest::DmSend {
            peer_key: to,
            body,
            reply_to,
        }
    }
}

host_command! {
    state;
    /// Retry a send by its client id (`IpcRequest::OutboxRetry`, plan E2.5):
    /// the stored row goes out under the same id, so no new row appears; its
    /// state arrives as `DmSendStatus`.
    pub async fn outbox_retry(
        client_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::OutboxRetry { client_id }
    }
}

host_command! {
    state;
    /// Discard a failed send by its client id (`IpcRequest::OutboxDiscard`).
    pub async fn outbox_discard(
        client_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::OutboxDiscard { client_id }
    }
}

host_command! {
    state;
    /// Report the draft to a peer (`IpcRequest::DmTyping`): `typing` is whether
    /// it holds text. The Host sends the start, its refresh and the idle stop
    /// (plan E2.3); typing is dropped, never queued, when there is no route.
    pub async fn send_typing(
        peer_id: String,
        typing: bool,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::DmTyping {
            peer_key: peer_id,
            typing,
        }
    }
}

host_command! {
    state;
    /// The newest `limit` messages with a friend before `before` (ms), oldest
    /// first (`IpcRequest::DmHistory`).
    pub async fn get_message_history(
        peer_id: String,
        limit: u32,
        before: Option<i64>,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<DmMessage>>, CommandError> {
        IpcRequest::DmHistory {
            peer_key: peer_id,
            limit,
            before,
        }
    }
}

host_command! {
    state;
    /// Mark everything from a friend read (`IpcRequest::DmMarkRead`), with the
    /// client id of the newest of their messages the focused window rendered.
    pub async fn mark_read(
        peer_id: String,
        rendered_through: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::DmMarkRead {
            peer_key: peer_id,
            rendered_through,
        }
    }
}
