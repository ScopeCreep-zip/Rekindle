use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::capnp_codec;
use crate::error::ProtocolError;
use crate::messaging::envelope::{MessageEnvelope, MessagePayload};

/// Parse a raw incoming message into a `MessageEnvelope`.
pub fn parse_envelope(data: &[u8]) -> Result<MessageEnvelope, ProtocolError> {
    capnp_codec::message::decode_envelope(data)
}

/// Verify the Ed25519 signature on a message envelope.
pub fn verify_envelope(envelope: &MessageEnvelope) -> Result<bool, ProtocolError> {
    // Reconstruct the signed data: timestamp || nonce || payload
    let mut signed_data = Vec::new();
    signed_data.extend_from_slice(&envelope.timestamp.to_le_bytes());
    signed_data.extend_from_slice(&envelope.nonce);
    signed_data.extend_from_slice(&envelope.payload);

    // Verify with ed25519-dalek
    let key_bytes: [u8; 32] = envelope
        .sender_key
        .as_slice()
        .try_into()
        .map_err(|_| ProtocolError::Verification("sender_key must be 32 bytes".into()))?;
    let verifying_key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| ProtocolError::Verification(format!("invalid sender key: {e}")))?;

    let sig_bytes: [u8; 64] = envelope
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| ProtocolError::Verification("signature must be 64 bytes".into()))?;
    let signature = Signature::from_bytes(&sig_bytes);

    match verifying_key.verify(&signed_data, &signature) {
        Ok(()) => {
            tracing::trace!(
                sender = hex::encode(&envelope.sender_key),
                "signature verified"
            );
            Ok(true)
        }
        Err(e) => {
            tracing::warn!(
                sender = hex::encode(&envelope.sender_key),
                error = %e,
                "signature verification failed"
            );
            Ok(false)
        }
    }
}

/// Deserialize the decrypted payload into a `MessagePayload` enum.
pub fn parse_payload(decrypted: &[u8]) -> Result<MessagePayload, ProtocolError> {
    serde_json::from_slice(decrypted)
        .map_err(|e| ProtocolError::Deserialization(format!("payload parse failed: {e}")))
}

/// Full incoming message processing pipeline:
/// 1. Parse envelope
/// 2. Verify signature
/// 3. Return envelope (decryption happens in the service layer)
pub fn process_incoming(raw: &[u8]) -> Result<MessageEnvelope, ProtocolError> {
    let envelope = parse_envelope(raw)?;
    let valid = verify_envelope(&envelope)?;
    if !valid {
        return Err(ProtocolError::Verification(
            "invalid envelope signature".into(),
        ));
    }
    Ok(envelope)
}
