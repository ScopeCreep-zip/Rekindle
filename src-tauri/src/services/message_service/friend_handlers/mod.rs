//! Phase 23.D — friend-handshake handlers lifted from
//! `message_service/mod.rs`. Owns the receive-side state machine for
//! incoming `FriendRequest` / `FriendAccept` / `FriendReject` /
//! `Unfriended` / `ProfileKeyRotated` payloads, plus the cross-request
//! auto-accept and the related SQLite cleanup helpers. Split into
//! focused submodules per Invariant 1 (≤500 LoC/file):
//!
//! * `session`   — primitive Signal-session install (request log + accept).
//! * `incoming`  — `FriendRequest` / `FriendAccept` full handlers with persist.
//! * `lifecycle` — `Unfriended` / `Reject` / cross-request auto-accept +
//!   `delete_pending_messages_to_recipient`.

mod incoming;
mod lifecycle;
mod session;

pub(super) use incoming::{handle_friend_accept_full, handle_friend_request_full};
pub(super) use lifecycle::{
    handle_friend_reject, handle_profile_key_rotated, handle_unfriended, handle_unfriended_ack,
};
pub(crate) use lifecycle::delete_pending_messages_to_recipient;

/// Consolidated parameters for an incoming friend request.
pub(super) struct IncomingFriendRequest<'a> {
    pub sender_hex: &'a str,
    pub display_name: &'a str,
    pub message: &'a str,
    pub prekey_bundle: &'a [u8],
    pub profile_dht_key: &'a str,
    pub route_blob: &'a [u8],
    pub mailbox_dht_key: &'a str,
    pub invite_id: Option<&'a str>,
}

/// Consolidated parameters for an incoming friend accept.
pub(super) struct IncomingFriendAccept<'a> {
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
