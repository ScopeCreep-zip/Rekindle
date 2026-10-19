//! The receive side of the friend handshake: incoming `FriendRequest`,
//! `FriendAccept`, `FriendReject`, `Unfriended`, `UnfriendedAck` and
//! `ProfileKeyRotated`, and the crossing-request auto-accept.
//!
//! * `incoming`  — the request and accept handlers, with their stores (the
//!   accept's session is the PreKeyInit that carried it, plan E2.3).
//! * `lifecycle` — reject, unfriend, rotation and the crossing request.

mod incoming;
mod lifecycle;

pub(crate) use incoming::{on_accept_failed, on_friend_accept, on_friend_request};
pub(crate) use lifecycle::{
    on_friend_reject, on_profile_key_rotated, on_unfriended, on_unfriended_ack,
};

/// An incoming friend request.
pub(crate) struct IncomingFriendRequest<'a> {
    pub sender_hex: &'a str,
    pub display_name: &'a str,
    pub message: &'a str,
    pub prekey_bundle: &'a [u8],
    pub profile_dht_key: &'a str,
    pub route_blob: &'a [u8],
    pub mailbox_dht_key: &'a str,
    pub invite_id: Option<&'a str>,
}

/// An incoming friend accept: the first payload of the PreKeyInit that
/// opened our session with them.
pub(crate) struct IncomingFriendAccept<'a> {
    pub sender_hex: &'a str,
    pub profile_dht_key: &'a str,
    pub route_blob: Vec<u8>,
    pub mailbox_dht_key: &'a str,
}
