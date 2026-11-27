//! The 1:1 send path on both hosts (plan E2.1, on `DmFrame` since E2.3):
//! sealing by carrier (`frame`), sends over the ports with their fallbacks,
//! the friend-handshake messages, our direct messages and our typing.

pub mod ack;
pub mod call_invite;
mod chain_seq;
mod chat;
pub mod disappearing;
mod first_contact_durable;
pub mod frame;
pub mod friend_control;
mod group_first_contact;
pub mod handshake;
mod handshake_order;
pub mod outbox;
mod outbox_payload;
mod outgoing;
pub(crate) mod receipts;
pub mod route;
pub(crate) mod route_health;
mod send;
pub mod stream;
mod typing;

pub use chat::{
    client_id_text, discard_send, history, inbox, mark_read, parse_client_id, retry_peer,
    retry_send, send_dm,
};
pub(crate) use chat::{mint_client_id, publish_dm_unread, NewDmFields};
pub use frame::wire;
pub use outgoing::{send_friend_accept, send_friend_request};
pub(crate) use send::framed;
pub use send::{hint_for, reply_frame, send_frame, send_to_peer, send_to_peer_call, Framed};
pub use typing::report_draft;

/// The ingress tag of a 1:1 message: the first byte of every 1:1 datagram
/// (§6 ingress ledger, `rekindle_codec::ingress`, beside the gossip tag),
/// so a receiver tells it from gossip and a transport frame without trying
/// to decode it.
pub use rekindle_codec::ingress::DIRECT_TAG;

/// The recipient's identity key a frame to `to` names.
///
/// # Errors
/// `Invalid` when `to` is not a public key.
pub(crate) fn recipient_key(to: &str) -> Result<[u8; 32], crate::operations::friend::FriendError> {
    rekindle_types::key_format::public_key_hex(to)
        .map(|k| k.to_bytes())
        .map_err(|e| {
            crate::operations::friend::FriendError::Invalid(format!("recipient key {to}: {e}"))
        })
}

/// The frame bytes of a 1:1 datagram, when it carries [`DIRECT_TAG`].
#[must_use]
pub fn unwire(datagram: &[u8]) -> Option<&[u8]> {
    match datagram.split_first() {
        Some((&DIRECT_TAG, rest)) => Some(rest),
        _ => None,
    }
}

/// How far past our receive time a received message's claimed time may
/// stand (`ENVELOPE_MAX_FUTURE_SKEW_MS`, the envelopes' allowed skew).
pub(crate) const MAX_FUTURE_SKEW_MS: i64 =
    rekindle_types::message::ENVELOPE_MAX_FUTURE_SKEW_MS.cast_signed();

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
