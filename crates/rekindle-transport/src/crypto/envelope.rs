//! Ed25519 envelope signing and verification.
//!
//! Used for all authenticated transport: gossip broadcasts, app_call RPCs,
//! and DM messages. Every inbound message is verified here before dispatch.
//! This is the fix for the unsigned app_call vulnerability.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rekindle_types::domains::DM_FRAME_SIG_V2;
use rekindle_types::message::{ENVELOPE_FRESHNESS_WINDOW_MS, ENVELOPE_MAX_FUTURE_SKEW_MS};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TransportError};
use crate::frame::TypeId;

/// A signed payload wrapper for DM and RPC messages.
///
/// The signature covers [`build_signed_data`]: the `DM_FRAME_SIG_V2`
/// domain, the recipient's identity key, the frame `TypeId`, the
/// timestamp, `seq`, the correlation id and the payload. Binding the
/// recipient keeps a captured envelope from being replayed to anyone else;
/// binding the `TypeId` keeps signed bytes from being re-framed as another
/// message type.
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
    seq: u64,
    correlation_id: Option<&str>,
    payload: &[u8],
) -> SignedPayload {
    let signing_key = SigningKey::from_bytes(sender_secret);
    let timestamp = rekindle_utils::timestamp_ms();

    let signed_data = build_signed_data(to, timestamp, seq, correlation_id, payload);
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
/// Layout: `DM_FRAME_SIG_V2 || recipient(32) || type_id(1) ||
/// timestamp(8 LE) || seq(8 LE) || correlation_id_len(4 LE) ||
/// correlation_id_bytes || payload`. `None` and `Some("")` correlation ids
/// sign identically; both mean "no group" for dedup.
fn build_signed_data(
    to: Addressing<'_>,
    timestamp: u64,
    seq: u64,
    correlation_id: Option<&str>,
    payload: &[u8],
) -> Vec<u8> {
    let correlation_bytes = correlation_id.map_or(&[][..], str::as_bytes);
    let correlation_len = u32::try_from(correlation_bytes.len()).unwrap_or(u32::MAX);

    let mut signed_data = Vec::with_capacity(
        DM_FRAME_SIG_V2.len() + 32 + 1 + 8 + 8 + 4 + correlation_bytes.len() + payload.len(),
    );
    signed_data.extend_from_slice(DM_FRAME_SIG_V2.as_bytes());
    signed_data.extend_from_slice(to.recipient);
    signed_data.push(to.type_id as u8);
    signed_data.extend_from_slice(&timestamp.to_le_bytes());
    signed_data.extend_from_slice(&seq.to_le_bytes());
    signed_data.extend_from_slice(&correlation_len.to_le_bytes());
    signed_data.extend_from_slice(correlation_bytes);
    signed_data.extend_from_slice(payload);
    signed_data
}

/// Verify a [`SignedPayload`] received as frame `to.type_id` by
/// `to.recipient` (our identity key): the signature, then the timestamp
/// against the shared freshness window.
pub fn verify_signed_payload(signed: &SignedPayload, to: Addressing<'_>) -> Result<()> {
    let verifying_key = parse_verifying_key(&signed.sender_key_hex)?;

    let signed_data = build_signed_data(
        to,
        signed.timestamp,
        signed.seq,
        signed.correlation_id.as_deref(),
        &signed.payload,
    );

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

    fn signed(seq: u64, correlation_id: Option<&str>, payload: &[u8]) -> SignedPayload {
        sign_payload(
            &SECRET,
            &signer_hex(),
            to_bob(TypeId::Unfriend),
            seq,
            correlation_id,
            payload,
        )
    }

    #[test]
    fn sign_verify_roundtrip() {
        let s = signed(1, None, b"test payload");
        assert!(verify_signed_payload(&s, to_bob(TypeId::Unfriend)).is_ok());
    }

    /// A captured envelope replayed to a third identity fails there.
    #[test]
    fn wrong_recipient_rejected() {
        let s = signed(1, None, b"unfriend");
        let to_carol = Addressing {
            recipient: &CAROL,
            type_id: TypeId::Unfriend,
        };
        assert!(verify_signed_payload(&s, to_carol).is_err());
    }

    /// Signed bytes re-framed under another `TypeId` fail.
    #[test]
    fn reframed_type_id_rejected() {
        let s = signed(1, None, b"payload");
        assert!(verify_signed_payload(&s, to_bob(TypeId::FriendReject)).is_err());
    }

    /// A signature over the pre-v2 layout (no domain, recipient or type)
    /// does not verify.
    #[test]
    fn v1_signature_rejected() {
        let mut s = signed(1, None, b"payload");
        let mut v1 = Vec::new();
        v1.extend_from_slice(&s.timestamp.to_le_bytes());
        v1.extend_from_slice(&s.seq.to_le_bytes());
        v1.extend_from_slice(&0u32.to_le_bytes());
        v1.extend_from_slice(&s.payload);
        s.signature = SigningKey::from_bytes(&SECRET)
            .sign(&v1)
            .to_bytes()
            .to_vec();
        assert!(verify_signed_payload(&s, to_bob(TypeId::Unfriend)).is_err());
    }

    #[test]
    fn stale_timestamp_rejected() {
        let to = to_bob(TypeId::Unfriend);
        let timestamp = rekindle_utils::timestamp_ms() - ENVELOPE_FRESHNESS_WINDOW_MS - 1_000;
        let signature = SigningKey::from_bytes(&SECRET)
            .sign(&build_signed_data(to, timestamp, 1, None, b"old"))
            .to_bytes()
            .to_vec();
        let s = SignedPayload {
            sender_key_hex: signer_hex(),
            timestamp,
            seq: 1,
            correlation_id: None,
            payload: b"old".to_vec(),
            signature,
        };
        assert!(verify_signed_payload(&s, to).is_err());
    }

    #[test]
    fn tampered_payload_rejected() {
        let mut s = signed(1, None, b"original");
        s.payload = b"tampered".to_vec();
        assert!(verify_signed_payload(&s, to_bob(TypeId::Unfriend)).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let mut s = signed(1, None, b"test");
        s.sender_key_hex = hex::encode(
            SigningKey::from_bytes(&[99u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        assert!(verify_signed_payload(&s, to_bob(TypeId::Unfriend)).is_err());
    }

    #[test]
    fn tampered_seq_rejected() {
        let mut s = signed(5, None, b"original");
        s.seq = 6; // forge a fresh seq to bypass dedup
        assert!(
            verify_signed_payload(&s, to_bob(TypeId::Unfriend)).is_err(),
            "tampered seq must fail signature verify",
        );
    }

    #[test]
    fn tampered_correlation_id_rejected() {
        let mut s = signed(1, Some("call-a"), b"original");
        s.correlation_id = Some("call-b".into());
        assert!(
            verify_signed_payload(&s, to_bob(TypeId::Unfriend)).is_err(),
            "tampered correlation_id must fail signature verify",
        );
    }

    #[test]
    fn correlation_id_round_trips() {
        let cid = "call-abc-123";
        let s = signed(7, Some(cid), b"x");
        assert!(verify_signed_payload(&s, to_bob(TypeId::Unfriend)).is_ok());
        assert_eq!(s.correlation_id.as_deref(), Some(cid));
        assert_eq!(s.seq, 7);
    }
}
