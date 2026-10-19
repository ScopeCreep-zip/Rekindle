//! Friend lifecycle events — request, accept, reject, remove, profile rotation.
//!
//! These are triggered by 1:1 kinds arriving as `'D'` envelopes on the
//! identity's own route.

use serde::{Deserialize, Serialize};

/// Friend lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FriendEvent {
    /// An inbound friend request was received.
    /// Triggered by: an inbound 1:1 `FriendRequest`.
    RequestReceived {
        from_key: String,
        display_name: String,
        message: String,
    },
    /// A friend request we sent was acknowledged (delivery confirmed).
    /// Triggered by: the receipt for our 1:1 `FriendRequest`.
    RequestAcknowledged { peer_key: String },
    /// A friend request was accepted (friendship established).
    /// Triggered by: an inbound 1:1 `FriendAccept`.
    Accepted {
        peer_key: String,
        /// The accepter's display name, when the acceptance carried it.
        /// The desktop's parallel event always had this and Tier 1 did
        /// not, so a CLI saw an acceptance from a bare key.
        display_name: Option<String>,
    },
    /// A friend request was rejected.
    /// Triggered by: an inbound 1:1 `FriendReject`.
    Rejected { peer_key: String },
    /// A friend row was added to our list — the point at which the
    /// friend becomes renderable, as distinct from [`Self::Accepted`],
    /// which is the protocol acknowledgement that precedes it.
    Added {
        peer_key: String,
        display_name: String,
        friendship_state: crate::friend_store::FriendshipState,
    },
    /// A friend removed us.
    /// Triggered by: an inbound 1:1 `Unfriended`.
    Removed { peer_key: String },
    /// Unfriend was acknowledged.
    /// Triggered by: an inbound 1:1 `UnfriendedAck`.
    RemoveAcknowledged { peer_key: String },
    /// A friend rotated their profile DHT key.
    /// Triggered by: an inbound 1:1 `ProfileKeyRotated`.
    ProfileKeyRotated {
        peer_key: String,
        new_profile_dht_key: String,
    },
    /// We set or cleared our local nickname for a friend.
    /// Triggered by: the `set_friend_nickname` action.
    NicknameChanged {
        peer_key: String,
        nickname: Option<String>,
    },
    /// A friend's group changed: moved, or their group renamed or deleted
    /// (`None`: no group).
    /// Triggered by: the friend-group actions.
    GroupChanged {
        peer_key: String,
        group: Option<String>,
    },
    /// The first-contact Signal handshake with a peer we sent a request to
    /// failed: no secure session exists, so the friendship stays pending.
    /// This is not a trust prompt; it never asks for "Reset Secure
    /// Session" (plan C7.25). An identity-key mismatch also lands here,
    /// with its own notice.
    /// Triggered by: a `FriendAccept` whose session init does not complete.
    HandshakeFailed { peer_key: String, reason: String },
}
