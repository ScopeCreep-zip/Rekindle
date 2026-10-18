//! Phase 23.B — extracted from `state.rs`. Per-friend state; our identity
//! and the status vocabulary are `rekindle_types::friend`'s (plan E1.1).

use serde::{Deserialize, Serialize};

pub use rekindle_types::friend::{IdentityState, UserStatus};

pub use rekindle_types::friend_store::FriendshipState;

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
    /// Why the first-contact Signal handshake failed, while it stays
    /// failed: the friendship remains pending and the buddy list says so
    /// (plan C7.25). Memory only; cleared when a session is established.
    pub handshake_error: Option<String>,
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

/// A Signal session reset we requested from a peer: the keys our
/// SessionResetRequest's handout bundle offered. Their SessionResetAccept
/// must name exactly these, or it was not an answer to our request
/// (plan C7.25).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutgoingReset {
    /// The signed prekey's id.
    pub signed_prekey: u32,
    /// The X25519 one-time prekey's id, if the bundle carried one.
    pub one_time_prekey: Option<u32>,
    /// The ML-KEM one-time key's id, if the bundle carried one.
    pub pq_one_time: Option<u32>,
}

impl OutgoingReset {
    /// Whether an accept's handshake ids answer this request.
    pub fn answered_by(&self, spk_id: u32, otpk_id: Option<u32>, pq_ot_id: Option<u32>) -> bool {
        self.signed_prekey == spk_id
            && self.one_time_prekey == otpk_id
            && self.pq_one_time == pq_ot_id
    }
}

#[cfg(test)]
mod outgoing_reset_tests {
    use super::OutgoingReset;

    #[test]
    fn only_the_offered_keys_answer_a_reset() {
        let sent = OutgoingReset {
            signed_prekey: 1,
            one_time_prekey: Some(5),
            pq_one_time: Some(6),
        };
        assert!(sent.answered_by(1, Some(5), Some(6)));
        assert!(!sent.answered_by(1, Some(5), None));
        assert!(!sent.answered_by(1, Some(9), Some(6)));
        assert!(!sent.answered_by(2, Some(5), Some(6)));
    }
}
