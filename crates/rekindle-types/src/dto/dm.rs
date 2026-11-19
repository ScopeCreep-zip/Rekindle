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
    /// The disappearing timer it carries (plan E7.1; 0 = none) and when it
    /// is deleted (ms), `None` while a received one waits to be displayed.
    pub expire_seconds: u32,
    pub expire_at: Option<i64>,
    /// A timer-change notice (plan E7.1): `sender_id` set the timer to
    /// this many seconds (0 = off); shown as a timeline line, never a
    /// bubble. `None` for a message.
    pub timer_notice_seconds: Option<u32>,
}

/// A DM's or group DM's disappearing-message timer as its chat window's
/// control shows it (`DisappearingGet`, plan E7.1): the timer in force,
/// the presets to offer and the longest custom value, all the Host's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisappearingTimerView {
    /// The timer in force, in seconds (0 = off).
    pub seconds: u32,
    /// [`crate::disappearing::PRESETS`].
    pub presets: Vec<u32>,
    /// [`crate::disappearing::MAX_SECONDS`].
    pub max_seconds: u32,
}

impl DisappearingTimerView {
    /// The control's view of a conversation whose timer is `seconds`.
    #[must_use]
    pub fn of(seconds: u32) -> Self {
        Self {
            seconds,
            presets: crate::disappearing::PRESETS.to_vec(),
            max_seconds: crate::disappearing::MAX_SECONDS,
        }
    }
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
