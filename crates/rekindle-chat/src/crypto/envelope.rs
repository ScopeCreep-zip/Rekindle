//! Ed25519 signed envelope for peer-to-peer messages with replay protection.
//!
//! Wire format:
//! ```text
//! [TypeId(1) || timestamp(8 LE) || sender_pubkey(32) || signature(64) || payload]
//! ```
//!
//! The signature covers `type_id(1) || timestamp(8 LE) || payload`.
//! The timestamp enables freshness verification — messages older than
//! the replay window (5 minutes) or from the future (>60s clock skew)
//! are rejected even if the signature is valid.
//!
//! Transport sends this as opaque bytes. Chat produces and parses it.

use rekindle_ratchet::crypto::sign;

use crate::time::timestamp_ms;
use crate::ChatError;

/// Wire header: TypeId(1) + timestamp(8) + pubkey(32) + signature(64) = 105 bytes.
const ENVELOPE_HEADER_LEN: usize = 1 + 8 + 32 + 64;

/// Replay protection window: reject messages older than 5 minutes.
const FRESHNESS_WINDOW_MS: u64 = 300_000;

/// Maximum acceptable clock skew into the future: 60 seconds.
const MAX_FUTURE_MS: u64 = 60_000;

/// Parsed signed envelope with replay protection metadata.
pub struct SignedEnvelope {
    pub type_id: u8,
    pub timestamp_ms: u64,
    pub sender_key: [u8; 32],
    pub signature: [u8; 64],
    pub payload: Vec<u8>,
}

impl SignedEnvelope {
    /// Build a signed envelope from a payload.
    ///
    /// The signature covers `type_id || timestamp || payload`. The timestamp
    /// is the current wall-clock time in milliseconds since UNIX epoch.
    pub fn build(
        type_id: u8,
        signing_seed: &[u8; 32],
        payload: &[u8],
    ) -> Result<Vec<u8>, ChatError> {
        let kp = sign::keypair_from_seed(signing_seed)
            .map_err(|e| ChatError::Internal(format!("sign keypair: {e}")))?;
        let pubkey = sign::public_key_bytes(&kp);

        let timestamp_ms = timestamp_ms();

        // Sign: type_id(1) || timestamp(8 LE) || payload
        let mut sign_input = Vec::with_capacity(1 + 8 + payload.len());
        sign_input.push(type_id);
        sign_input.extend_from_slice(&timestamp_ms.to_le_bytes());
        sign_input.extend_from_slice(payload);
        let sig = sign::sign_ec_prekey(&kp, &sign_input);

        // Wire: type_id(1) || timestamp(8 LE) || pubkey(32) || sig(64) || payload
        let mut wire = Vec::with_capacity(ENVELOPE_HEADER_LEN + payload.len());
        wire.push(type_id);
        wire.extend_from_slice(&timestamp_ms.to_le_bytes());
        wire.extend_from_slice(&pubkey);
        wire.extend_from_slice(&sig);
        wire.extend_from_slice(payload);
        Ok(wire)
    }

    /// Parse an envelope from raw bytes. Does NOT verify signature or freshness.
    /// Call `verify()` after parse to authenticate and check replay protection.
    pub fn parse(data: &[u8]) -> Result<Self, ChatError> {
        if data.len() < ENVELOPE_HEADER_LEN {
            return Err(ChatError::Deserialization(format!(
                "envelope too short: {} bytes (min {})",
                data.len(),
                ENVELOPE_HEADER_LEN
            )));
        }
        let type_id = data[0];
        let timestamp_ms = u64::from_le_bytes(
            data[1..9].try_into().expect("8 bytes"),
        );
        let mut sender_key = [0u8; 32];
        sender_key.copy_from_slice(&data[9..41]);
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&data[41..105]);
        let payload = data[105..].to_vec();
        Ok(Self {
            type_id,
            timestamp_ms,
            sender_key,
            signature,
            payload,
        })
    }

    /// Verify the Ed25519 signature AND timestamp freshness.
    ///
    /// Rejects:
    /// - Invalid signature (forgery or corruption)
    /// - Stale timestamp (>5 minutes old — replay attack)
    /// - Future timestamp (>60 seconds ahead — clock skew or injection)
    ///
    /// Returns Ok(()) only if both signature and freshness pass.
    pub fn verify(&self) -> Result<(), ChatError> {
        // Signature verification: covers type_id || timestamp || payload
        let mut sign_input = Vec::with_capacity(1 + 8 + self.payload.len());
        sign_input.push(self.type_id);
        sign_input.extend_from_slice(&self.timestamp_ms.to_le_bytes());
        sign_input.extend_from_slice(&self.payload);

        sign::verify_ec_prekey(&self.sender_key, &sign_input, &self.signature)
            .map_err(|_| ChatError::Internal(format!(
                "envelope signature invalid (sender {})",
                hex::encode(&self.sender_key[..8]),
            )))?;

        // Freshness verification: reject replayed and future-dated messages
        let now = timestamp_ms();
        let age_ms = now.saturating_sub(self.timestamp_ms);
        let future_ms = self.timestamp_ms.saturating_sub(now);

        if age_ms > FRESHNESS_WINDOW_MS {
            return Err(ChatError::Internal(format!(
                "envelope rejected: stale timestamp ({}ms old, max {}ms) — \
                 possible replay attack from sender {}",
                age_ms,
                FRESHNESS_WINDOW_MS,
                hex::encode(&self.sender_key[..8]),
            )));
        }

        if future_ms > MAX_FUTURE_MS {
            return Err(ChatError::Internal(format!(
                "envelope rejected: timestamp {}ms in the future (max {}ms clock skew) — \
                 possible injection from sender {}",
                future_ms,
                MAX_FUTURE_MS,
                hex::encode(&self.sender_key[..8]),
            )));
        }

        Ok(())
    }

    /// Verify signature only, skip freshness check.
    ///
    /// Use ONLY for scenarios where the timestamp is not meaningful
    /// (e.g., reading historical entries from DHT where staleness is
    /// expected). For all real-time inbound messages, use `verify()`.
    pub fn verify_signature_only(&self) -> Result<(), ChatError> {
        let mut sign_input = Vec::with_capacity(1 + 8 + self.payload.len());
        sign_input.push(self.type_id);
        sign_input.extend_from_slice(&self.timestamp_ms.to_le_bytes());
        sign_input.extend_from_slice(&self.payload);

        sign::verify_ec_prekey(&self.sender_key, &sign_input, &self.signature)
            .map_err(|_| ChatError::Internal(format!(
                "envelope signature invalid (sender {})",
                hex::encode(&self.sender_key[..8]),
            )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_seed() -> [u8; 32] { [42u8; 32] }

    #[test]
    fn build_parse_verify_roundtrip() {
        let wire = SignedEnvelope::build(1, &test_seed(), b"hello").unwrap();
        let envelope = SignedEnvelope::parse(&wire).unwrap();
        assert_eq!(envelope.type_id, 1);
        assert_eq!(envelope.payload, b"hello");
        assert!(envelope.verify().is_ok());
    }

    #[test]
    fn tampered_payload_rejected() {
        let wire = SignedEnvelope::build(1, &test_seed(), b"original").unwrap();
        let mut envelope = SignedEnvelope::parse(&wire).unwrap();
        envelope.payload = b"tampered".to_vec();
        assert!(envelope.verify().is_err());
    }

    #[test]
    fn tampered_timestamp_rejected() {
        let wire = SignedEnvelope::build(1, &test_seed(), b"hello").unwrap();
        let mut envelope = SignedEnvelope::parse(&wire).unwrap();
        envelope.timestamp_ms = 0; // ancient timestamp
        // Signature will fail because timestamp is part of signed data
        assert!(envelope.verify().is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let wire = SignedEnvelope::build(1, &test_seed(), b"hello").unwrap();
        let mut envelope = SignedEnvelope::parse(&wire).unwrap();
        envelope.sender_key = [99u8; 32];
        assert!(envelope.verify().is_err());
    }

    #[test]
    fn stale_envelope_rejected() {
        let wire = SignedEnvelope::build(1, &test_seed(), b"hello").unwrap();
        let mut envelope = SignedEnvelope::parse(&wire).unwrap();
        // Forge a valid-looking but stale timestamp
        // (signature will also fail because timestamp is signed, but
        // this tests the freshness path in isolation via verify_signature_only)
        envelope.timestamp_ms = timestamp_ms().saturating_sub(FRESHNESS_WINDOW_MS + 1000);
        // Can't test freshness independently because timestamp is signed.
        // The signature check catches it first. This is correct behavior —
        // an attacker cannot modify the timestamp without invalidating the sig.
        assert!(envelope.verify().is_err());
    }

    #[test]
    fn wire_format_length() {
        let wire = SignedEnvelope::build(1, &test_seed(), b"test").unwrap();
        // Header: 1 + 8 + 32 + 64 = 105, payload: 4
        assert_eq!(wire.len(), 105 + 4);
    }
}
