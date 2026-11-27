//! The receive side of the friend handshake: incoming `FriendRequest`,
//! `FriendAccept`, `Unfriended`, `UnfriendedAck` and
//! `ProfileKeyRotated`, and the crossing-request auto-accept.
//!
//! * `incoming`  — the request and accept handlers, with their stores (the
//!   accept's session is the PreKeyInit that carried it, plan E2.3).
//! * `lifecycle` — unfriend, rotation and the crossing request.

mod incoming;
mod lifecycle;

pub(crate) use incoming::{on_accept_failed, on_friend_accept, on_friend_request};
pub(crate) use lifecycle::{on_profile_key_rotated, on_unfriended, on_unfriended_ack};

/// An incoming friend request.
pub(crate) struct IncomingFriendRequest<'a> {
    pub sender_hex: &'a str,
    pub display_name: &'a str,
    pub message: &'a str,
    pub profile_dht_key: &'a str,
    pub route_blob: &'a [u8],
    pub mailbox_dht_key: &'a str,
    /// The friend-code token or the invite that admits it.
    pub auth: &'a rekindle_codec::capnp_codec::direct::RequestAuth,
    /// The requester's reply inbox (plan RV2).
    pub reply: &'a rekindle_codec::message::envelope::InboxCapability,
    /// The requester's dm chain record to us (decision 67 item 2), adopted
    /// only once we accept.
    pub chain_record_key: &'a str,
}

/// An incoming friend accept: the first payload of the PreKeyInit that
/// opened our session with them.
pub(crate) struct IncomingFriendAccept<'a> {
    pub sender_hex: &'a str,
    pub profile_dht_key: &'a str,
    pub route_blob: Vec<u8>,
    pub mailbox_dht_key: &'a str,
    /// The accepter's presence key (decision 67 item 2).
    pub friends_key: &'a rekindle_codec::capnp_codec::direct::FriendsKey,
    /// The accepter's dm chain record to us (decision 67 item 2).
    pub chain_record_key: &'a str,
}
