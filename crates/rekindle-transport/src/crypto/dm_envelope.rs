//! DM envelope construction and verification: an Ed25519-signed,
//! recipient-bound envelope around a DM payload.
//!
//! **The daemon track does not encrypt DMs today** (audit V7). The payload this
//! module signs is not Signal ciphertext: `broadcast/dm.rs` either appends the
//! body hex-encoded to a DHT log or sends it as signed-only postcard. The
//! signature gives integrity and sender authenticity, not confidentiality, so a
//! daemon-track DM is readable by relays and storage nodes. Plan step E2.1
//! deletes this path in favour of the desktop's ported Signal runtime, and E2.3
//! moves DMs onto ratchet-sealed `DmFrame`s.
//!
//! Send: payload → Ed25519 sign (this module) → frame encode → Veilid
//! app_message. Receive: frame decode → signature verify (`dispatch.rs` calls
//! `crypto/envelope.rs`) → deserialize to `DmPayload`.

use crate::crypto::envelope::{sign_payload, Addressing, SignedPayload};

/// Build a signed DM envelope from pre-encrypted Signal ciphertext.
///
/// The `ciphertext` parameter is the output of Signal Protocol encryption.
/// This function wraps it in a [`SignedPayload`] with the sender's
/// Ed25519 identity key signature for envelope-level integrity.
///
/// The type-specific DM payload (DirectMessage, FriendRequest, etc.) is
/// serialized by the caller and passed as `ciphertext`. For message types
/// that don't use Signal encryption (FriendRequest, FriendAccept), the
/// payload is plaintext serialized bytes — the signature still provides
/// integrity and sender authentication.
///
/// `to` names the recipient's identity key and the frame `TypeId`; the
/// signature is bound to both.
///
/// W16.3 — `seq` and `correlation_id` are envelope-level metadata for
/// the receiver-side dedup primitive. Callers from outside the queue
/// (e.g. one-shot DM body sends) pass `seq=0`, `correlation_id=None`;
/// callers from inside `EnvelopeQueue` pass the row's allocated values.
pub fn build_dm_envelope(
    sender_secret: &[u8; 32],
    sender_public_hex: &str,
    to: Addressing<'_>,
    seq: u64,
    correlation_id: Option<&str>,
    payload_bytes: &[u8],
) -> SignedPayload {
    sign_payload(
        sender_secret,
        sender_public_hex,
        to,
        seq,
        correlation_id,
        payload_bytes,
    )
}

/// Extract the inner payload bytes from a verified DM envelope.
///
/// This is a trivial accessor — the heavy lifting (signature verification)
/// is done in `dispatch.rs` before this is called. The returned bytes
/// are either Signal-encrypted ciphertext (for session-based messages)
/// or plaintext serialized payload (for session-establishing messages
/// like FriendRequest).
pub fn extract_payload(signed: &SignedPayload) -> &[u8] {
    &signed.payload
}
