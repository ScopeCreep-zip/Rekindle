//! Signed gossip envelope construction and verification.
//!
//! Every gossip message is wrapped in a `SignedEnvelope` containing:
//! - The community ID (governance record key)
//! - The sender's pseudonym (hex-encoded Ed25519 public key)
//! - A 128-bit message id, unique per send (the dedup key)
//! - The serialized inner envelope bytes
//! - An Ed25519 signature over the id and those bytes
//! - A TTL hop counter (default 5, decremented on each forward)
//!
//! The id names one *message*, not its content: the dedup cache keys by
//! it, a relay forwards it unchanged (so one message's flood is
//! suppressed), and a deliberate re-send of identical content (a roster
//! reconcile, a retry) is a new message with a new id, so it is
//! delivered. Receivers are idempotent. This is the libp2p pubsub
//! spec's origin-stamped id (`from` + a per-origin unique `seqno`,
//! covered by the signature); it warns that a content-hash id drops an
//! identical re-send as a duplicate. The id is random from the host's
//! `RandomSource` rather than a counter: a counter must survive restarts
//! (or reuse ids peers still hold as seen) and reveals how many messages
//! a pseudonym has sent.

use rekindle_secrets::derive::{derive_community_pseudonym, sign_with_pseudonym};
use rekindle_secrets::sign::verify_signature;
use rekindle_types::error::CryptoError;
use serde::{Deserialize, Serialize};

/// Default TTL for gossip messages — maximum 5 hops through the mesh.
pub const DEFAULT_TTL: u8 = 5;

/// Length of a gossip message id.
pub const MESSAGE_ID_LEN: usize = 16;

/// Domain tag prefixed to every gossip envelope signature, so a
/// pseudonym's gossip signature can never be read as a signature over
/// any other structure that key signs.
const SIGNATURE_DOMAIN: &[u8] = b"rekindle-gossip-envelope:v1:";

/// The exact bytes a gossip envelope's signature covers:
/// `SIGNATURE_DOMAIN ‖ message_id ‖ envelope_bytes`. The id is fixed
/// length, so the concatenation cannot alias.
#[must_use]
pub fn signing_input(message_id: &[u8; MESSAGE_ID_LEN], envelope_bytes: &[u8]) -> Vec<u8> {
    [SIGNATURE_DOMAIN, message_id.as_slice(), envelope_bytes].concat()
}

/// A signed gossip envelope ready for network transmission.
///
/// The `envelope_bytes` field contains a JSON-serialized `GovernanceEntry`,
/// `ChannelEntry`, or other community message type. The signature covers
/// exactly these bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedEnvelope {
    /// DHT key of the community's governance record.
    pub community_id: String,
    /// Hex-encoded Ed25519 public key of the sender's community pseudonym.
    pub sender_pseudonym: String,
    /// Unique per send, chosen by the sender, covered by the signature and
    /// forwarded unchanged by relays: the dedup key.
    pub message_id: [u8; MESSAGE_ID_LEN],
    /// Serialized inner payload (signed data).
    pub envelope_bytes: Vec<u8>,
    /// Ed25519 signature over [`signing_input`]`(message_id, envelope_bytes)`.
    pub signature: Vec<u8>,
    /// Hop TTL — starts at DEFAULT_TTL, decremented on each gossip forward.
    /// When 0, process locally but don't forward.
    #[serde(default = "default_ttl")]
    pub ttl: u8,
}

fn default_ttl() -> u8 {
    DEFAULT_TTL
}

/// Build and sign a gossip envelope.
///
/// Serializes the payload to JSON bytes, signs with the sender's pseudonym
/// Ed25519 key, and wraps in a `SignedEnvelope`.
///
/// # Arguments
/// * `master_secret` — The user's master secret (for pseudonym derivation).
/// * `community_id` — The governance record DHT key.
/// * `message_id` — A fresh id for this send (see the module doc).
/// * `payload` — Any serializable payload (GovernanceEntry, ChannelEntry, etc.)
pub fn build_signed_envelope<T: Serialize>(
    master_secret: &[u8; 32],
    community_id: &str,
    message_id: [u8; MESSAGE_ID_LEN],
    payload: &T,
) -> Result<SignedEnvelope, rekindle_types::error::CommunityError> {
    let signing_key = derive_community_pseudonym(master_secret, community_id);
    let sender_pseudonym = hex::encode(signing_key.verifying_key().to_bytes());
    let envelope_bytes = serde_json::to_vec(payload)?;
    let signature = sign_with_pseudonym(&signing_key, &signing_input(&message_id, &envelope_bytes));

    Ok(SignedEnvelope {
        community_id: community_id.to_string(),
        sender_pseudonym,
        message_id,
        envelope_bytes,
        signature: signature.to_vec(),
        ttl: DEFAULT_TTL,
    })
}

/// Verify the Ed25519 signature on a signed envelope.
///
/// The `sender_pseudonym` field is hex-encoded. Returns `Ok(())` if valid.
pub fn verify_signed_envelope(signed: &SignedEnvelope) -> Result<(), CryptoError> {
    let pub_bytes = hex::decode(&signed.sender_pseudonym)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid pseudonym hex: {e}")))?;
    let pub_array: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| CryptoError::InvalidKey("pseudonym key must be 32 bytes".into()))?;
    let sig_array: [u8; 64] = signed
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Verification("signature must be 64 bytes".into()))?;

    verify_signature(
        &pub_array,
        &signing_input(&signed.message_id, &signed.envelope_bytes),
        &sig_array,
    )
}

/// Deserialize the inner payload from a verified signed envelope.
pub fn deserialize_payload<T: for<'de> Deserialize<'de>>(
    signed: &SignedEnvelope,
) -> Result<T, rekindle_types::error::CommunityError> {
    serde_json::from_slice(&signed.envelope_bytes).map_err(Into::into)
}

/// Create a forwarding copy with decremented TTL.
///
/// Returns `None` if TTL is already 0 (don't forward).
pub fn forward_envelope(signed: &SignedEnvelope) -> Option<SignedEnvelope> {
    let ttl = signed.ttl.checked_sub(1)?;
    let mut forwarded = signed.clone();
    forwarded.ttl = ttl;
    Some(forwarded)
}

#[cfg(test)]
#[path = "envelope_tests.rs"]
mod tests;
