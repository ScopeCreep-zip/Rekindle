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
        /// The shared DM log spine, when the acceptance established one.
        /// `Option` rather than an empty string: the DM-payload path
        /// genuinely does not carry it, and `String::new()` there was
        /// indistinguishable from a real-but-empty key.
        dm_log_key: Option<String>,
        /// The accepter's display name, when the acceptance carried it.
        /// The desktop's parallel event always had this and Tier 1 did
        /// not, so a CLI saw an acceptance from a bare key.
        display_name: Option<String>,
    },
    /// A friend request was rejected.
    /// Triggered by: `DmPayload::FriendReject`, DHT watch on friend inbox (Rejected status).
    Rejected { peer_key: String },
    /// A friend row was added to our list — the point at which the
    /// friend becomes renderable, as distinct from [`Self::Accepted`],
    /// which is the protocol acknowledgement that precedes it.
    Added {
        peer_key: String,
        display_name: String,
        /// Serialized `FriendshipState` — "pending", "active", etc.
        friendship_state: String,
    },
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
