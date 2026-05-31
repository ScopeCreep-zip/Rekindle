//! Phase 23.B — extracted from `state.rs`. Per-friend state +
//! identity + status enums.

use serde::{Deserialize, Serialize};

/// The logged-in user's identity state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityState {
    pub public_key: String,
    pub display_name: String,
    pub status: UserStatus,
    pub status_message: String,
}

/// Online status enum matching Xfire's status system.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    #[default]
    Online,
    Away,
    Busy,
    Offline,
    /// Invisible: the user is online but appears offline to others.
    /// They can still receive messages and see who's online.
    Invisible,
}

/// Whether a friendship is pending (outbound request sent) or fully accepted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FriendshipState {
    PendingOut,
    #[default]
    Accepted,
    /// Friend removal in progress — kept in state so `sync_service` can retry
    /// the `Unfriended` notification using the friend's routing info.
    /// Hidden from the UI via `get_friends` filtering.
    Removing,
}

/// A friend's state as seen on the buddy list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendState {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub status: UserStatus,
    pub status_message: Option<String>,
    pub game_info: Option<GameInfoState>,
    pub group: Option<String>,
    pub unread_count: u32,
    /// The friend's DHT profile record key for presence watching.
    pub dht_record_key: Option<String>,
    /// Unix timestamp (ms) of when this friend was last seen online.
    pub last_seen_at: Option<i64>,
    /// Our local conversation DHT record key for this friend.
    pub local_conversation_key: Option<String>,
    /// The friend's conversation DHT record key (their side).
    pub remote_conversation_key: Option<String>,
    /// The friend's mailbox DHT key (for route discovery).
    pub mailbox_dht_key: Option<String>,
    /// Unix timestamp (ms) from the friend's last DHT heartbeat.
    /// Used for stale presence detection.
    pub last_heartbeat_at: Option<i64>,
    /// Whether this friendship is pending (request sent) or fully accepted.
    pub friendship_state: FriendshipState,
}

/// Game presence information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoState {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    /// Direct server address ("ip:port") for join-game functionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_address: Option<String>,
}
