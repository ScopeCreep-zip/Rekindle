//! Direct-message DTOs (plan E2.1): what `DmHistory` and `DmInbox` answer.

use serde::{Deserialize, Serialize};

use crate::message::SendState;

/// One direct message, as a chat window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmMessage {
    pub id: i64,
    /// Its client id (a UUID v7 its author minted), which a reply names;
    /// `None` for a row stored before E2.3.
    pub message_id: Option<String>,
    pub sender_id: String,
    pub body: String,
    /// When it was sent, ms since the epoch.
    pub timestamp: i64,
    pub is_own: bool,
    /// Our own message's delivery state; `delivered` for a received one.
    pub send_state: SendState,
    /// A history-gap marker (plan E2.5, D23): the peer's entries through
    /// this seq were retired before we fetched them; shown as a gap line,
    /// never a bubble. `None` for a message.
    pub history_gap_through: Option<u64>,
}

/// The latest message of one direct conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmInboxEntry {
    pub peer_key: String,
    pub sender_key: String,
    pub body: String,
    pub timestamp: i64,
    pub is_read: bool,
}
