//! Ed25519 signing and verification of the daemon's framed community
//! RPCs (`app_call`). Every inbound framed call is verified here before
//! dispatch. 1:1 messages are not framed: each is a `'D'` `DmFrame`, sealed
//! by its carrier (`operations::direct`, plan E2.3).

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rekindle_types::domains::FRAME_SIG_V3;
use rekindle_types::message::{ENVELOPE_FRESHNESS_WINDOW_MS, ENVELOPE_MAX_FUTURE_SKEW_MS};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TransportError};
use crate::frame::TypeId;

/// A signed payload wrapper for framed community RPCs.
///
/// The signature covers [`build_signed_data`]: the `FRAME_SIG_V3`
/// domain, the recipient's identity key, the frame `TypeId`, the
/// timestamp and the payload. Binding the recipient keeps a captured
/// frame from being replayed to anyone else; binding the `TypeId` keeps
/// signed bytes from being re-framed as another message type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedPayload {
    /// Sender's Ed25519 public key (hex-encoded, 64 chars).
    pub sender_key_hex: String,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// The serialized inner payload (type-specific).
    pub payload: Vec<u8>,
    /// Ed25519 signature over [`build_signed_data`].
    pub signature: Vec<u8>,
}

/// What a [`SignedPayload`] is signed for: the recipient's identity key
/// and the frame type it travels under.
#[derive(Debug, Clone, Copy)]
pub struct Addressing<'a> {
    pub recipient: &'a [u8; 32],
    pub type_id: TypeId,
}

/// Sign a payload with the sender's Ed25519 secret key, timestamped now,
/// for `to.recipient` under frame type `to.type_id`.
pub fn sign_payload(
    sender_secret: &[u8; 32],
    sender_public_hex: &str,
    to: Addressing<'_>,
    payload: &[u8],
) -> SignedPayload {
    let signing_key = SigningKey::from_bytes(sender_secret);
    let timestamp = rekindle_utils::timestamp_ms();

    let signed_data = build_signed_data(to, timestamp, payload);
    let signature = signing_key.sign(&signed_data);

    SignedPayload {
        sender_key_hex: sender_public_hex.to_string(),
        timestamp,
        payload: payload.to_vec(),
        signature: signature.to_bytes().to_vec(),
    }
}

/// Build the byte sequence that the Ed25519 signature covers.
///
/// Layout: `FRAME_SIG_V3 || recipient(32) || type_id(1) ||
/// timestamp(8 LE) || payload`.
fn build_signed_data(to: Addressing<'_>, timestamp: u64, payload: &[u8]) -> Vec<u8> {
    let mut signed_data = Vec::with_capacity(FRAME_SIG_V3.len() + 32 + 1 + 8 + payload.len());
    signed_data.extend_from_slice(FRAME_SIG_V3.as_bytes());
    signed_data.extend_from_slice(to.recipient);
    signed_data.push(to.type_id as u8);
    signed_data.extend_from_slice(&timestamp.to_le_bytes());
    signed_data.extend_from_slice(payload);
    signed_data
}

/// Verify a [`SignedPayload`] received as frame `to.type_id` by
/// `to.recipient` (our identity key): the signature, then the timestamp
/// against the shared freshness window.
pub fn verify_signed_payload(signed: &SignedPayload, to: Addressing<'_>) -> Result<()> {
    let verifying_key = parse_verifying_key(&signed.sender_key_hex)?;

    let signed_data = build_signed_data(to, signed.timestamp, &signed.payload);

    let signature = parse_signature(&signed.signature)?;

    verifying_key
        .verify_strict(&signed_data, &signature)
        .map_err(|_| TransportError::SignatureVerificationFailed {
            sender: signed.sender_key_hex.clone(),
        })?;

    let now = rekindle_utils::timestamp_ms();
    let age_ms = now.saturating_sub(signed.timestamp);
    if age_ms > ENVELOPE_FRESHNESS_WINDOW_MS {
        return Err(TransportError::SignatureVerificationFailed {
            sender: format!(
                "{}: stale timestamp ({age_ms}ms old, window {ENVELOPE_FRESHNESS_WINDOW_MS}ms)",
                signed.sender_key_hex
            ),
        });
    }
    let future_ms = signed.timestamp.saturating_sub(now);
    if future_ms > ENVELOPE_MAX_FUTURE_SKEW_MS {
        return Err(TransportError::SignatureVerificationFailed {
            sender: format!(
                "{}: timestamp {future_ms}ms in the future (max {ENVELOPE_MAX_FUTURE_SKEW_MS}ms)",
                signed.sender_key_hex
            ),
        });
    }

    Ok(())
}

/// The 32-byte identity key a hex public key names, for [`Addressing`].
pub fn recipient_bytes(public_key_hex: &str) -> Result<[u8; 32]> {
    rekindle_types::key_format::public_key_hex(public_key_hex)
        .map(|k| k.to_bytes())
        .map_err(|e| TransportError::Internal(format!("recipient key {public_key_hex}: {e}")))
}

// ── Helpers ──────────────────────────────────────────────────────────

fn parse_verifying_key(hex_str: &str) -> Result<VerifyingKey> {
    let bytes = hex::decode(hex_str).map_err(|e| TransportError::SignatureVerificationFailed {
        sender: format!("invalid hex: {e}"),
    })?;
    let arr: [u8; 32] =
        bytes
            .try_into()
            .map_err(|_| TransportError::SignatureVerificationFailed {
                sender: "public key must be 32 bytes".into(),
            })?;
    VerifyingKey::from_bytes(&arr).map_err(|e| TransportError::SignatureVerificationFailed {
        sender: format!("invalid Ed25519 key: {e}"),
    })
}

fn parse_signature(sig_bytes: &[u8]) -> Result<Signature> {
    let arr: [u8; 64] =
        sig_bytes
            .try_into()
            .map_err(|_| TransportError::SignatureVerificationFailed {
                sender: "signature must be 64 bytes".into(),
            })?;
    Ok(Signature::from_bytes(&arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: [u8; 32] = [42u8; 32];
    const BOB: [u8; 32] = [7u8; 32];
    const CAROL: [u8; 32] = [9u8; 32];

    fn to_bob(type_id: TypeId) -> Addressing<'static> {
        Addressing {
            recipient: &BOB,
            type_id,
        }
    }

    fn signer_hex() -> String {
        hex::encode(SigningKey::from_bytes(&SECRET).verifying_key().to_bytes())
    }

    fn signed(payload: &[u8]) -> SignedPayload {
        sign_payload(
            &SECRET,
            &signer_hex(),
            to_bob(TypeId::CommunityLeave),
            payload,
        )
    }

    #[test]
    fn sign_verify_roundtrip() {
        let s = signed(b"test payload");
        assert!(verify_signed_payload(&s, to_bob(TypeId::CommunityLeave)).is_ok());
    }

    /// A captured frame replayed to a third identity fails there.
    #[test]
    fn wrong_recipient_rejected() {
        let s = signed(b"leave");
        let to_carol = Addressing {
            recipient: &CAROL,
            type_id: TypeId::CommunityLeave,
        };
        assert!(verify_signed_payload(&s, to_carol).is_err());
    }

    /// Signed bytes re-framed under another `TypeId` fail.
    #[test]
    fn reframed_type_id_rejected() {
        let s = signed(b"payload");
        assert!(verify_signed_payload(&s, to_bob(TypeId::SyncRequest)).is_err());
    }

    /// A signature over the v2 field layout (`seq` and a correlation-id
    /// length after the timestamp) does not verify.
    #[test]
    fn v2_layout_rejected() {
        let mut s = signed(b"payload");
        let mut v2 = Vec::new();
        v2.extend_from_slice(FRAME_SIG_V3.as_bytes());
        v2.extend_from_slice(&BOB);
        v2.push(TypeId::CommunityLeave as u8);
        v2.extend_from_slice(&s.timestamp.to_le_bytes());
        v2.extend_from_slice(&0u64.to_le_bytes());
        v2.extend_from_slice(&0u32.to_le_bytes());
        v2.extend_from_slice(&s.payload);
        s.signature = SigningKey::from_bytes(&SECRET)
            .sign(&v2)
            .to_bytes()
            .to_vec();
        assert!(verify_signed_payload(&s, to_bob(TypeId::CommunityLeave)).is_err());
    }

    #[test]
    fn stale_timestamp_rejected() {
        let to = to_bob(TypeId::CommunityLeave);
        let timestamp = rekindle_utils::timestamp_ms() - ENVELOPE_FRESHNESS_WINDOW_MS - 1_000;
        let signature = SigningKey::from_bytes(&SECRET)
            .sign(&build_signed_data(to, timestamp, b"old"))
            .to_bytes()
            .to_vec();
        let s = SignedPayload {
            sender_key_hex: signer_hex(),
            timestamp,
            payload: b"old".to_vec(),
            signature,
        };
        assert!(verify_signed_payload(&s, to).is_err());
    }

    #[test]
    fn tampered_payload_rejected() {
        let mut s = signed(b"original");
        s.payload = b"tampered".to_vec();
        assert!(verify_signed_payload(&s, to_bob(TypeId::CommunityLeave)).is_err());
    }

    #[test]
    fn tampered_timestamp_rejected() {
        let mut s = signed(b"original");
        s.timestamp -= 1;
        assert!(verify_signed_payload(&s, to_bob(TypeId::CommunityLeave)).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let mut s = signed(b"test");
        s.sender_key_hex = hex::encode(
            SigningKey::from_bytes(&[99u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        assert!(verify_signed_payload(&s, to_bob(TypeId::CommunityLeave)).is_err());
    }
}
