use ed25519_dalek::{Signer, SigningKey};

use rekindle_codec::message::envelope::MessageEnvelope;
use rekindle_codec::message::signing::envelope_signing_bytes;

/// Build and sign a `MessageEnvelope` to `recipient` from raw secret key
/// bytes. See [`build_envelope`].
pub fn build_envelope_from_secret(
    secret_key_bytes: &[u8; 32],
    recipient: &[u8; 32],
    timestamp: u64,
    nonce: Vec<u8>,
    payload: Vec<u8>,
) -> MessageEnvelope {
    let signing_key = SigningKey::from_bytes(secret_key_bytes);
    build_envelope(&signing_key, recipient, timestamp, nonce, payload)
}

/// Build and sign a `MessageEnvelope` to `recipient` (their identity key).
///
/// The signature covers [`envelope_signing_bytes`], which binds the
/// recipient, so the envelope verifies only for them. A retry passes the
/// original nonce and payload with a fresh timestamp.
pub fn build_envelope(
    signing_key: &SigningKey,
    recipient: &[u8; 32],
    timestamp: u64,
    nonce: Vec<u8>,
    payload: Vec<u8>,
) -> MessageEnvelope {
    let sender_key = signing_key.verifying_key().to_bytes().to_vec();
    let signature = signing_key.sign(&envelope_signing_bytes(
        recipient, timestamp, &nonce, &payload,
    ));

    MessageEnvelope {
        sender_key,
        timestamp,
        nonce,
        payload,
        signature: signature.to_bytes().to_vec(),
    }
}
