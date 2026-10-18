//! Our own presence (plan E1.1): what `PresenceGet` answers, for the
//! status picker, the game panel and the TUI.

use serde::{Deserialize, Serialize};

use crate::presence::{IdleState, SessionStatus};

/// The status, message, game and idle state the host holds for us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnPresenceView {
    pub status: SessionStatus,
    pub status_message: String,
    pub game: Option<OwnGame>,
    pub idle: IdleState,
}

/// The game we are playing, detected or set by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnGame {
    pub game_id: u32,
    pub game_name: String,
    /// Seconds since the game started.
    pub elapsed_seconds: u32,
    /// The server to join, `host:port`, already parsed.
    pub server_address: Option<String>,
}
