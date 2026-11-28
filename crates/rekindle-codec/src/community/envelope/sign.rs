//! Signing and verification of community envelopes.

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};

use crate::envelope::{signing_input, DEFAULT_TTL, MESSAGE_ID_LEN};

use super::SignedEnvelope;

/// Create a signed envelope from a serialized envelope payload.
///
/// Signs `message_id ‖ envelope_bytes` (under the gossip domain tag,
/// [`signing_input`]) with the pseudonym's Ed25519 signing key.
/// `message_id` is fresh per send, from the host's `RandomSource`: a
/// re-send of the same bytes is a new message (see
/// `crate::envelope`'s module doc).
pub fn sign_envelope(
    signing_key: &SigningKey,
    community_id: &str,
    sender_pseudonym: &str,
    message_id: [u8; MESSAGE_ID_LEN],
    envelope_bytes: &[u8],
) -> SignedEnvelope {
    let signature = signing_key.sign(&signing_input(&message_id, envelope_bytes));
    SignedEnvelope {
        community_id: community_id.to_string(),
        sender_pseudonym: sender_pseudonym.to_string(),
        message_id,
        envelope_bytes: envelope_bytes.to_vec(),
        signature: signature.to_bytes().to_vec(),
        ttl: DEFAULT_TTL,
    }
}

/// Verify the Ed25519 signature on a signed envelope.
///
/// Delegates to `crate::envelope::verify_signed_envelope` —
/// this crate carried a second copy of the same checks (hex-decode the
/// pseudonym, require 32-byte key and 64-byte signature, Ed25519
/// verify). The `String` error is kept because callers here match on it.
pub fn verify_envelope(signed: &SignedEnvelope) -> Result<(), String> {
    crate::envelope::verify_signed_envelope(signed).map_err(|e| e.to_string())
}
