//! DM envelope construction and verification.
//!
//! DM messages use Signal Protocol for encryption (handled by `rekindle-secrets`)
//! and Ed25519 for envelope-level integrity. This module builds the outer
//! envelope that wraps the Signal-encrypted ciphertext.
//!
//! The crypto flow is:
//! 1. Plaintext → Signal Protocol encrypt (caller, via rekindle-secrets)
//! 2. Ciphertext → Ed25519 sign (this module)
//! 3. Signed envelope → frame encode → Veilid app_message
//!
//! On receive:
//! 1. Frame decode → signature verify (dispatch.rs calls crypto/envelope.rs)
//! 2. Ciphertext → Signal Protocol decrypt (caller, via rekindle-secrets)
//! 3. Plaintext → deserialize to DmPayload

use crate::crypto::envelope::{sign_payload, SignedPayload};

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
/// W16.3 — `seq` and `correlation_id` are envelope-level metadata for
/// the receiver-side dedup primitive. Callers from outside the queue
/// (e.g. one-shot DM body sends) pass `seq=0`, `correlation_id=None`;
/// callers from inside `EnvelopeQueue` pass the row's allocated values.
pub fn build_dm_envelope(
    sender_secret: &[u8; 32],
    sender_public_hex: &str,
    seq: u64,
    correlation_id: Option<&str>,
    payload_bytes: &[u8],
) -> SignedPayload {
    sign_payload(sender_secret, sender_public_hex, seq, correlation_id, payload_bytes)
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
