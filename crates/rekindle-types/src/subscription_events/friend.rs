//! Friend lifecycle events — request, accept, reject, remove, profile rotation.
//!
//! These are triggered by `DmPayload` variants and DHT `ValueChange`
//! on the friend inbox record.

use serde::{Deserialize, Serialize};

/// Friend lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FriendEvent {
    /// We sent a friend request (dispatch-emitted for subscriber visibility).
    /// Triggered by: dispatch after successful `send_friend_request()`.
    RequestSent {
        target_profile_key: String,
        dm_log_key: String,
    },
    /// An inbound friend request was received.
    /// Triggered by: DHT watch on friend inbox (signed FriendRequestEntry).
    RequestReceived {
        /// Ed25519 public key, 64 hex chars.
        from_key: String,
        display_name: String,
        message: String,
    },
    /// A friend request we sent was acknowledged (delivery confirmed).
    /// Triggered by: `DmPayload::FriendRequestAck` app_message.
    RequestAcknowledged {
        /// Ed25519 public key, 64 hex chars.
        peer_key: String,
    },
    /// A friend request was accepted (friendship established).
    /// Triggered by: DHT inbox scan discovering FriendRequestStatus::Accepted.
    Accepted {
        /// Ed25519 public key, 64 hex chars.
        peer_key: String,
        dm_log_key: String,
    },
    /// A friend request was rejected.
    /// Triggered by: DHT inbox scan discovering FriendRequestStatus::Rejected.
    Rejected {
        peer_key: String,
    },
    /// A friend removed us.
    /// Triggered by: `DmPayload::Unfriend`.
    Removed {
        peer_key: String,
    },
    /// Unfriend was acknowledged.
    /// Triggered by: `DmPayload::UnfriendAck`.
    RemoveAcknowledged {
        peer_key: String,
    },
    /// A friend rotated their profile DHT key.
    /// Triggered by: `DmPayload::ProfileKeyRotated`.
    ProfileKeyRotated {
        peer_key: String,
        new_profile_dht_key: String,
    },
}
