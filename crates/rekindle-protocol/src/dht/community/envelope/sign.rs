//! Signing and verification of community envelopes.

use ed25519_dalek::{Signer, SigningKey};

use rekindle_codec::envelope::DEFAULT_TTL;

use super::SignedEnvelope;

/// Create a signed envelope from a serialized envelope payload.
///
/// Signs `envelope_bytes` with the pseudonym's Ed25519 signing key.
pub fn sign_envelope(
    signing_key: &SigningKey,
    community_id: &str,
    sender_pseudonym: &str,
    envelope_bytes: &[u8],
) -> SignedEnvelope {
    let signature = signing_key.sign(envelope_bytes);
    SignedEnvelope {
        community_id: community_id.to_string(),
        sender_pseudonym: sender_pseudonym.to_string(),
        envelope_bytes: envelope_bytes.to_vec(),
        signature: signature.to_bytes().to_vec(),
        ttl: DEFAULT_TTL,
    }
}

/// Verify the Ed25519 signature on a signed envelope.
///
/// Delegates to `rekindle_codec::envelope::verify_signed_envelope` —
/// this crate carried a second copy of the same checks (hex-decode the
/// pseudonym, require 32-byte key and 64-byte signature, Ed25519
/// verify). The `String` error is kept because callers here match on it.
pub fn verify_envelope(signed: &SignedEnvelope) -> Result<(), String> {
    rekindle_codec::envelope::verify_signed_envelope(signed).map_err(|e| e.to_string())
}
