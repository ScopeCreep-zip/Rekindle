//! Our own identity as the frontends show it, the friends the host holds,
//! and the one status vocabulary a friend and we are described in (plans
//! E1.1, E2.1). `UserStatus` is
//! [`SessionStatus`](crate::presence::SessionStatus): friends and community
//! presence speak one language, Invisible folding to Offline on the wire.

use serde::{Deserialize, Serialize};

pub use crate::presence::SessionStatus as UserStatus;

/// The unlocked identity: who we are and the status we show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityState {
    pub public_key: String,
    pub display_name: String,
    pub status: UserStatus,
    pub status_message: String,
}

pub use crate::friend_store::FriendshipState;

/// A friend as the host holds it (plan E2.1): the stored row, and what the
/// friend's presence and our conversations say now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The friend's profile record, which we watch for their presence.
    pub dht_record_key: Option<String>,
    /// When the friend was last seen online, ms since the epoch.
    pub last_seen_at: Option<i64>,
    /// The friend's mailbox, where their route is published.
    pub mailbox_dht_key: Option<String>,
    /// The friend's last presence heartbeat, ms since the epoch: a
    /// stale one reads as offline.
    pub last_heartbeat_at: Option<i64>,
    pub friendship_state: FriendshipState,
    /// Why the first-contact handshake failed, while it stays failed
    /// (plan C7.25). Not stored; cleared when a session is established.
    pub handshake_error: Option<String>,
    /// Our first contact is being resent from a fresh key after the peer
    /// rejected the one it used (plan E2.4, B6). Not stored.
    pub handshake_retrying: bool,
    /// Their prekey record is not signed by the identity we hold for them
    /// (`IdentityPinMismatch`, plan E2.4 B3): no session opens until the
    /// user resolves it. Not stored; it stays set until a record of theirs
    /// verifies, and the next read raises it again after an unlock.
    pub identity_pin_mismatch: bool,
}

impl FriendState {
    /// A friend with no presence seen yet: offline, nothing unread.
    #[must_use]
    pub fn new(
        public_key: String,
        display_name: String,
        friendship_state: FriendshipState,
    ) -> Self {
        Self {
            public_key,
            display_name,
            nickname: None,
            status: UserStatus::Offline,
            status_message: None,
            game_info: None,
            group: None,
            unread_count: 0,
            dht_record_key: None,
            last_seen_at: None,
            mailbox_dht_key: None,
            last_heartbeat_at: None,
            friendship_state,
            handshake_error: None,
            handshake_retrying: false,
            identity_pin_mismatch: false,
        }
    }
}

/// What a friend is playing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoState {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    /// The server to join, `host:port`, parsed on ingest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_address: Option<String>,
}
