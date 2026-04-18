//! Signed gossip envelope construction and verification.
//!
//! Every gossip message is wrapped in a `SignedEnvelope` containing:
//! - The community ID (governance record key)
//! - The sender's pseudonym (hex-encoded Ed25519 public key)
//! - The serialized inner envelope bytes
//! - An Ed25519 signature over those bytes
//! - A TTL hop counter (default 5, decremented on each forward)

use rekindle_secrets::derive::{sign_with_pseudonym, derive_community_pseudonym};
use rekindle_secrets::sign::verify_signature;
use rekindle_types::error::CryptoError;
use serde::{Deserialize, Serialize};

/// Default TTL for gossip messages — maximum 5 hops through the mesh.
pub const DEFAULT_TTL: u8 = 5;

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
    /// Serialized inner payload (signed data).
    pub envelope_bytes: Vec<u8>,
    /// Ed25519 signature over `envelope_bytes`.
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
/// * `payload` — Any serializable payload (GovernanceEntry, ChannelEntry, etc.)
pub fn build_signed_envelope<T: Serialize>(
    master_secret: &[u8; 32],
    community_id: &str,
    payload: &T,
) -> Result<SignedEnvelope, rekindle_types::error::CommunityError> {
    let signing_key = derive_community_pseudonym(master_secret, community_id);
    let sender_pseudonym = hex::encode(signing_key.verifying_key().to_bytes());
    let envelope_bytes = serde_json::to_vec(payload)?;
    let signature = sign_with_pseudonym(&signing_key, &envelope_bytes);

    Ok(SignedEnvelope {
        community_id: community_id.to_string(),
        sender_pseudonym,
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

    verify_signature(&pub_array, &signed.envelope_bytes, &sig_array)
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
    if signed.ttl == 0 {
        return None;
    }
    let mut forwarded = signed.clone();
    forwarded.ttl = signed.ttl.saturating_sub(1);
    Some(forwarded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::governance::GovernanceEntry;
    use rekindle_types::id::ChannelId;

    #[test]
    fn sign_verify_roundtrip() {
        let secret = [42u8; 32];
        let community_id = "VLD0:test_community";
        let payload = GovernanceEntry::CommunityMeta {
            name: Some("Test".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        };

        let signed = build_signed_envelope(&secret, community_id, &payload).unwrap();
        assert!(verify_signed_envelope(&signed).is_ok());
    }

    #[test]
    fn tampered_payload_fails_verify() {
        let secret = [42u8; 32];
        let payload = GovernanceEntry::MEKGenerationBump {
            generation: 1,
            lamport: 1,
        };

        let mut signed = build_signed_envelope(&secret, "c", &payload).unwrap();
        // Tamper with the payload
        signed.envelope_bytes[0] ^= 0xFF;
        assert!(verify_signed_envelope(&signed).is_err());
    }

    #[test]
    fn wrong_sender_fails_verify() {
        let secret = [42u8; 32];
        let payload = GovernanceEntry::MEKGenerationBump {
            generation: 1,
            lamport: 1,
        };

        let mut signed = build_signed_envelope(&secret, "c", &payload).unwrap();
        // Replace sender with a different pseudonym
        let wrong_key = rekindle_secrets::derive::derive_community_pseudonym(&[99u8; 32], "c");
        signed.sender_pseudonym = hex::encode(wrong_key.verifying_key().to_bytes());
        assert!(verify_signed_envelope(&signed).is_err());
    }

    #[test]
    fn deserialize_payload_roundtrip() {
        let secret = [42u8; 32];
        let payload = GovernanceEntry::ChannelCreated {
            channel_id: ChannelId([1u8; 16]),
            name: "general".into(),
            channel_type: "text".into(),
            record_key: "VLD0:abc".into(),
            category_id: None,
            position: 0,
            lamport: 5,
        };

        let signed = build_signed_envelope(&secret, "c", &payload).unwrap();
        let deserialized: GovernanceEntry = deserialize_payload(&signed).unwrap();
        assert_eq!(payload, deserialized);
    }

    #[test]
    fn forward_decrements_ttl() {
        let secret = [42u8; 32];
        let payload = GovernanceEntry::MEKGenerationBump {
            generation: 1,
            lamport: 1,
        };
        let signed = build_signed_envelope(&secret, "c", &payload).unwrap();
        assert_eq!(signed.ttl, DEFAULT_TTL);

        let fwd = forward_envelope(&signed).unwrap();
        assert_eq!(fwd.ttl, DEFAULT_TTL - 1);
        // Signature unchanged
        assert_eq!(fwd.signature, signed.signature);
    }

    #[test]
    fn forward_at_zero_returns_none() {
        let mut signed = build_signed_envelope(
            &[1u8; 32],
            "c",
            &GovernanceEntry::MEKGenerationBump {
                generation: 1,
                lamport: 1,
            },
        )
        .unwrap();
        signed.ttl = 0;
        assert!(forward_envelope(&signed).is_none());
    }
}
