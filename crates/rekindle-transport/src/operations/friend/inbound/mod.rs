//! The receive side of the friend handshake: incoming `FriendRequest`,
//! `FriendAccept`, `FriendReject`, `Unfriended`, `UnfriendedAck` and
//! `ProfileKeyRotated`, and the crossing-request auto-accept.
//!
//! * `session`   — the responder-side Signal session from an accept.
//! * `incoming`  — the request and accept handlers, with their stores.
//! * `lifecycle` — reject, unfriend, rotation and the crossing request.

mod incoming;
mod lifecycle;
mod session;

pub(crate) use incoming::{on_friend_accept, on_friend_request};
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

/// An incoming friend accept.
pub(crate) struct IncomingFriendAccept<'a> {
    pub sender_hex: &'a str,
    pub prekey_bundle: &'a [u8],
    pub profile_dht_key: &'a str,
    pub route_blob: Vec<u8>,
    pub mailbox_dht_key: &'a str,
    pub ephemeral_key: &'a [u8],
    pub signed_prekey_id: u32,
    pub one_time_prekey_id: Option<u32>,
    pub ml_kem_ciphertext: &'a [u8],
    pub used_ot_pqpk_id: Option<u32>,
}
