//! Friend lifecycle events — request, accept, reject, remove, profile rotation.
//!
//! These are triggered by `DmPayload` variants and DHT `ValueChange`
//! on the friend inbox record.

use serde::{Deserialize, Serialize};

/// Friend lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FriendEvent {
    /// An inbound friend request was received.
    /// Triggered by: `DmPayload::FriendRequest`, DHT watch on friend inbox.
    RequestReceived {
        from_key: String,
        display_name: String,
        message: String,
    },
    /// A friend request we sent was acknowledged (delivery confirmed).
    /// Triggered by: `DmPayload::FriendRequestAck`.
    RequestAcknowledged { peer_key: String },
    /// A friend request was accepted (friendship established).
    /// Triggered by: `DmPayload::FriendAccept`, DHT watch on friend inbox (Accepted status).
    Accepted {
        peer_key: String,
        dm_log_key: String,
    },
    /// A friend request was rejected.
    /// Triggered by: `DmPayload::FriendReject`, DHT watch on friend inbox (Rejected status).
    Rejected { peer_key: String },
    /// A friend removed us.
    /// Triggered by: `DmPayload::Unfriend`.
    Removed { peer_key: String },
    /// Unfriend was acknowledged.
    /// Triggered by: `DmPayload::UnfriendAck`.
    RemoveAcknowledged { peer_key: String },
    /// A friend rotated their profile DHT key.
    /// Triggered by: `DmPayload::ProfileKeyRotated`.
    ProfileKeyRotated {
        peer_key: String,
        new_profile_dht_key: String,
    },
}
