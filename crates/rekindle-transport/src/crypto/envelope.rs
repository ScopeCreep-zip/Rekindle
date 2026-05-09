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
/// The signature covers `timestamp(8 LE) || seq(8 LE) || correlation_id_len(4 LE) || correlation_id_bytes || payload`.
/// The sender's Ed25519 public key is included for verification.
///
/// W16.3: `seq` and `correlation_id` are envelope-level metadata used by
/// the receiver-side dedup primitive (`SeqTracker`). The signature
/// covers them so they can't be forged after the wire — a peer can't
/// inject a duplicate envelope with a fresh seq to bypass dedup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedPayload {
    /// Sender's Ed25519 public key (hex-encoded, 64 chars).
    pub sender_key_hex: String,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// W16.3 — per-recipient sequence allocated by `EnvelopeQueue`.
    /// Receiver tracks the last-seen seq per (sender, kind, correlation_id)
    /// and drops envelopes with `seq <= last_seen` as duplicates.
    pub seq: u64,
    /// W16.3 — optional grouping key. For call envelopes this is the
    /// `call_id`; for DM invites the request's correlation_id; `None`
    /// for envelopes that aren't part of a logical group.
    pub correlation_id: Option<String>,
    /// The serialized inner payload (type-specific).
    pub payload: Vec<u8>,
    /// Ed25519 signature over `timestamp(8 LE) || seq(8 LE) ||
    /// correlation_id_len(4 LE) || correlation_id_bytes || payload`.
    pub signature: Vec<u8>,
}

/// Sign a payload with the sender's Ed25519 secret key.
///
/// Produces a [`SignedPayload`] with the signature covering timestamp,
/// seq, correlation_id, and payload.
pub fn sign_payload(
    sender_secret: &[u8; 32],
    sender_public_hex: &str,
    seq: u64,
    correlation_id: Option<&str>,
    payload: &[u8],
) -> SignedPayload {
    let signing_key = SigningKey::from_bytes(sender_secret);
    let timestamp = rekindle_utils::timestamp_ms();

    let signed_data = build_signed_data(timestamp, seq, correlation_id, payload);
    let signature = signing_key.sign(&signed_data);

    SignedPayload {
        sender_key_hex: sender_public_hex.to_string(),
        timestamp,
        seq,
        correlation_id: correlation_id.map(str::to_string),
        payload: payload.to_vec(),
        signature: signature.to_bytes().to_vec(),
    }
}

/// Build the byte sequence that the Ed25519 signature covers.
///
/// Layout: `timestamp(8 LE) || seq(8 LE) || correlation_id_len(4 LE) ||
/// correlation_id_bytes || payload`.
///
/// `correlation_id_len` is `0` when `correlation_id` is `None`, otherwise
/// the UTF-8 byte length. This makes the absence of a correlation_id
/// distinguishable from an empty-string correlation_id (both serialize
/// as zero bytes but with different lengths) — well, `None` serializes
/// to `0` and `Some("")` serializes to `0` followed by zero bytes; they
/// produce identical signed_data. That's intentional: an empty-string
/// correlation_id is equivalent to None for dedup purposes.
fn build_signed_data(
    timestamp: u64,
    seq: u64,
    correlation_id: Option<&str>,
    payload: &[u8],
) -> Vec<u8> {
    let correlation_bytes = correlation_id.map_or(&[][..], str::as_bytes);
    let correlation_len = u32::try_from(correlation_bytes.len()).unwrap_or(u32::MAX);

    let mut signed_data = Vec::with_capacity(8 + 8 + 4 + correlation_bytes.len() + payload.len());
    signed_data.extend_from_slice(&timestamp.to_le_bytes());
    signed_data.extend_from_slice(&seq.to_le_bytes());
    signed_data.extend_from_slice(&correlation_len.to_le_bytes());
    signed_data.extend_from_slice(correlation_bytes);
    signed_data.extend_from_slice(payload);
    signed_data
}

/// Default replay protection window: 5 minutes (300 seconds).
///
/// Messages with timestamps older than this are rejected even if the
/// signature is valid. This prevents indefinite replay of captured messages.
/// Set to 0 to disable freshness checking (not recommended).
pub const DEFAULT_FRESHNESS_WINDOW_MS: u64 = 300_000;

/// Verify the Ed25519 signature and timestamp freshness on a [`SignedPayload`].
///
/// Returns `Ok(())` if the signature is valid AND the timestamp is within
/// the freshness window. Rejects stale messages to prevent replay attacks.
pub fn verify_signed_payload(signed: &SignedPayload) -> Result<()> {
    verify_signed_payload_with_window(signed, DEFAULT_FRESHNESS_WINDOW_MS)
}

/// Verify with a custom freshness window. Pass 0 to skip freshness check.
pub fn verify_signed_payload_with_window(signed: &SignedPayload, freshness_window_ms: u64) -> Result<()> {
    // Signature verification first
    let verifying_key = parse_verifying_key(&signed.sender_key_hex)?;

    let signed_data = build_signed_data(
        signed.timestamp,
        signed.seq,
        signed.correlation_id.as_deref(),
        &signed.payload,
    );

    let signature = parse_signature(&signed.signature)?;

    verifying_key.verify(&signed_data, &signature).map_err(|_| {
        TransportError::SignatureVerificationFailed {
            sender: signed.sender_key_hex.clone(),
        }
    })?;

    // Freshness check — reject replayed messages
    if freshness_window_ms > 0 {
        let now = rekindle_utils::timestamp_ms();
        let age_ms = now.saturating_sub(signed.timestamp);
        // Also reject messages from the future (clock skew > 60s)
        let future_ms = signed.timestamp.saturating_sub(now);
        if age_ms > freshness_window_ms {
            return Err(TransportError::SignatureVerificationFailed {
                sender: format!(
                    "{}: stale timestamp ({}ms old, window {}ms)",
                    signed.sender_key_hex, age_ms, freshness_window_ms
                ),
            });
        }
        if future_ms > 60_000 {
            return Err(TransportError::SignatureVerificationFailed {
                sender: format!(
                    "{}: timestamp {}ms in the future (max 60s clock skew allowed)",
                    signed.sender_key_hex, future_ms
                ),
            });
        }
    }

    Ok(())
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

        let signed = sign_payload(&secret, &public_hex, 1, None, b"test payload");
        assert!(verify_signed_payload(&signed).is_ok());
    }

    #[test]
    fn tampered_payload_rejected() {
        let secret = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let public_hex = hex::encode(signing_key.verifying_key().to_bytes());

        let mut signed = sign_payload(&secret, &public_hex, 1, None, b"original");
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

        let mut signed = sign_payload(
            &secret1,
            &hex::encode(key1.verifying_key().to_bytes()),
            1,
            None,
            b"test",
        );
        signed.sender_key_hex = public_hex2;
        assert!(verify_signed_payload(&signed).is_err());
    }

    #[test]
    fn tampered_seq_rejected() {
        let secret = [42u8; 32];
        let key = SigningKey::from_bytes(&secret);
        let public_hex = hex::encode(key.verifying_key().to_bytes());

        let mut signed = sign_payload(&secret, &public_hex, 5, None, b"original");
        signed.seq = 6; // forge a fresh seq to bypass dedup
        assert!(
            verify_signed_payload(&signed).is_err(),
            "tampered seq must fail signature verify",
        );
    }

    #[test]
    fn tampered_correlation_id_rejected() {
        let secret = [42u8; 32];
        let key = SigningKey::from_bytes(&secret);
        let public_hex = hex::encode(key.verifying_key().to_bytes());

        let mut signed = sign_payload(&secret, &public_hex, 1, Some("call-a"), b"original");
        signed.correlation_id = Some("call-b".into());
        assert!(
            verify_signed_payload(&signed).is_err(),
            "tampered correlation_id must fail signature verify",
        );
    }

    #[test]
    fn correlation_id_round_trips() {
        let secret = [42u8; 32];
        let key = SigningKey::from_bytes(&secret);
        let public_hex = hex::encode(key.verifying_key().to_bytes());

        let cid = "call-abc-123";
        let signed = sign_payload(&secret, &public_hex, 7, Some(cid), b"x");
        assert!(verify_signed_payload(&signed).is_ok());
        assert_eq!(signed.correlation_id.as_deref(), Some(cid));
        assert_eq!(signed.seq, 7);
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
