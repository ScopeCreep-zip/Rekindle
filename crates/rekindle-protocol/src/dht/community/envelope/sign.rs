//! Signing and verification of community envelopes.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use super::{default_ttl, SignedEnvelope};

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
        ttl: default_ttl(),
    }
}

/// Verify the Ed25519 signature on a signed envelope.
///
/// The `sender_pseudonym` field is the hex-encoded Ed25519 public key.
/// Returns `Ok(())` if the signature is valid.
pub fn verify_envelope(signed: &SignedEnvelope) -> Result<(), String> {
    let pub_bytes =
        hex::decode(&signed.sender_pseudonym).map_err(|e| format!("invalid pseudonym hex: {e}"))?;
    let pub_array: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| "pseudonym key must be 32 bytes".to_string())?;
    let verifying_key =
        VerifyingKey::from_bytes(&pub_array).map_err(|e| format!("invalid public key: {e}"))?;

    let sig_array: [u8; 64] = signed
        .signature
        .clone()
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;
    let signature = Signature::from_bytes(&sig_array);

    verifying_key
        .verify_strict(&signed.envelope_bytes, &signature)
        .map_err(|e| format!("invalid envelope signature: {e}"))
}
