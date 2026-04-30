//! Ed25519 envelope signing and verification.
//!
//! Used for all authenticated transport: gossip broadcasts, app_call RPCs,
//! and DM messages. Every inbound message is verified here before dispatch.
//! This is the fix for the unsigned app_call vulnerability.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::error::{TransportError, Result};
use crate::payload::gossip::SignedGossipEnvelope;

/// A signed payload wrapper for DM and RPC messages.
///
/// The signature covers `timestamp_bytes(8 LE) || payload`.
/// The sender's Ed25519 public key is included for verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedPayload {
    /// Sender's Ed25519 public key (hex-encoded, 64 chars).
    pub sender_key_hex: String,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// The serialized inner payload (type-specific).
    pub payload: Vec<u8>,
    /// Ed25519 signature over `timestamp(8 LE) || payload`.
    pub signature: Vec<u8>,
}

/// Sign a payload with the sender's Ed25519 secret key.
///
/// Produces a [`SignedPayload`] with the signature computed over
/// `timestamp(8 LE) || payload`.
pub fn sign_payload(
    sender_secret: &[u8; 32],
    sender_public_hex: &str,
    payload: &[u8],
) -> SignedPayload {
    let signing_key = SigningKey::from_bytes(sender_secret);
    let timestamp = rekindle_utils::timestamp_ms();

    let mut signed_data = Vec::with_capacity(8 + payload.len());
    signed_data.extend_from_slice(&timestamp.to_le_bytes());
    signed_data.extend_from_slice(payload);

    let signature = signing_key.sign(&signed_data);

    SignedPayload {
        sender_key_hex: sender_public_hex.to_string(),
        timestamp,
        payload: payload.to_vec(),
        signature: signature.to_bytes().to_vec(),
    }
}

/// Verify the Ed25519 signature on a [`SignedPayload`].
///
/// Returns `Ok(())` if valid. Returns `Err(SignatureVerificationFailed)` otherwise.
pub fn verify_signed_payload(signed: &SignedPayload) -> Result<()> {
    let verifying_key = parse_verifying_key(&signed.sender_key_hex)?;

    let mut signed_data = Vec::with_capacity(8 + signed.payload.len());
    signed_data.extend_from_slice(&signed.timestamp.to_le_bytes());
    signed_data.extend_from_slice(&signed.payload);

    let signature = parse_signature(&signed.signature)?;

    verifying_key.verify(&signed_data, &signature).map_err(|_| {
        TransportError::SignatureVerificationFailed {
            sender: signed.sender_key_hex.clone(),
        }
    })
}

/// Sign a gossip envelope with the sender's pseudonym key.
///
/// Signature is computed over `payload_bytes` only (the inner serialized
/// gossip payload). Community ID and sender pseudonym are in the clear
/// for routing/dedup but are not signed — the payload itself carries
/// the authenticated content.
pub fn sign_gossip_envelope(
    signing_key: &SigningKey,
    community_id: &str,
    sender_pseudonym: &str,
    payload_bytes: &[u8],
    ttl: u8,
    lamport_ts: u64,
) -> SignedGossipEnvelope {
    let signature = signing_key.sign(payload_bytes);

    SignedGossipEnvelope {
        community_id: community_id.to_string(),
        sender_pseudonym: sender_pseudonym.to_string(),
        payload_bytes: payload_bytes.to_vec(),
        signature: signature.to_bytes().to_vec(),
        ttl,
        lamport_ts,
    }
}

/// Verify the Ed25519 signature on a [`SignedGossipEnvelope`].
///
/// The `sender_pseudonym` field is the hex-encoded Ed25519 public key.
pub fn verify_gossip_envelope(envelope: &SignedGossipEnvelope) -> Result<()> {
    let verifying_key = parse_verifying_key(&envelope.sender_pseudonym)?;
    let signature = parse_signature(&envelope.signature)?;

    verifying_key
        .verify(&envelope.payload_bytes, &signature)
        .map_err(|_| TransportError::SignatureVerificationFailed {
            sender: envelope.sender_pseudonym.clone(),
        })
}

// ── Helpers ──────────────────────────────────────────────────────────

fn parse_verifying_key(hex_str: &str) -> Result<VerifyingKey> {
    let bytes = hex::decode(hex_str).map_err(|e| TransportError::SignatureVerificationFailed {
        sender: format!("invalid hex: {e}"),
    })?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| {
        TransportError::SignatureVerificationFailed {
            sender: "public key must be 32 bytes".into(),
        }
    })?;
    VerifyingKey::from_bytes(&arr).map_err(|e| TransportError::SignatureVerificationFailed {
        sender: format!("invalid Ed25519 key: {e}"),
    })
}

fn parse_signature(sig_bytes: &[u8]) -> Result<Signature> {
    let arr: [u8; 64] = sig_bytes.try_into().map_err(|_| {
        TransportError::SignatureVerificationFailed {
            sender: "signature must be 64 bytes".into(),
        }
    })?;
    Ok(Signature::from_bytes(&arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let secret = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let public_hex = hex::encode(signing_key.verifying_key().to_bytes());

        let signed = sign_payload(&secret, &public_hex, b"test payload");
        assert!(verify_signed_payload(&signed).is_ok());
    }

    #[test]
    fn tampered_payload_rejected() {
        let secret = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let public_hex = hex::encode(signing_key.verifying_key().to_bytes());

        let mut signed = sign_payload(&secret, &public_hex, b"original");
        signed.payload = b"tampered".to_vec();
        assert!(verify_signed_payload(&signed).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let secret1 = [42u8; 32];
        let secret2 = [99u8; 32];
        let key1 = SigningKey::from_bytes(&secret1);
        let key2 = SigningKey::from_bytes(&secret2);
        let public_hex2 = hex::encode(key2.verifying_key().to_bytes());

        let mut signed = sign_payload(&secret1, &hex::encode(key1.verifying_key().to_bytes()), b"test");
        signed.sender_key_hex = public_hex2;
        assert!(verify_signed_payload(&signed).is_err());
    }

    #[test]
    fn gossip_envelope_sign_verify() {
        let secret = [7u8; 32];
        let key = SigningKey::from_bytes(&secret);
        let pseudo_hex = hex::encode(key.verifying_key().to_bytes());

        let envelope = sign_gossip_envelope(
            &key,
            "community_abc",
            &pseudo_hex,
            b"gossip payload",
            5,
            42,
        );

        assert!(verify_gossip_envelope(&envelope).is_ok());
    }
}
