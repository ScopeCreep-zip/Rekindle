use ed25519_dalek::{Signature, VerifyingKey};
use rekindle_types::message::{ENVELOPE_FRESHNESS_WINDOW_MS, ENVELOPE_MAX_FUTURE_SKEW_MS};

use crate::capnp_codec;
use crate::error::ProtocolError;
use crate::messaging::envelope::{MessageEnvelope, MessagePayload, Sealing};
use crate::messaging::signing::envelope_signing_bytes;

/// Parse a raw incoming message into a `MessageEnvelope`.
pub fn parse_envelope(data: &[u8]) -> Result<MessageEnvelope, ProtocolError> {
    capnp_codec::message::decode_envelope(data)
}

/// Verify the envelope's Ed25519 signature as addressed to
/// `expected_recipient` (our identity key). An envelope signed for anyone
/// else, or in the pre-v2 format, fails.
pub fn verify_envelope(
    envelope: &MessageEnvelope,
    expected_recipient: &[u8; 32],
) -> Result<(), ProtocolError> {
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

    let signed = envelope_signing_bytes(
        expected_recipient,
        envelope.timestamp,
        &envelope.nonce,
        &envelope.payload,
    );
    verifying_key
        .verify_strict(&signed, &signature)
        .map_err(|e| ProtocolError::Verification(format!("invalid envelope signature: {e}")))
}

/// Reject an envelope whose timestamp is outside the freshness window
/// around `now_ms`.
pub fn check_freshness(timestamp: u64, now_ms: u64) -> Result<(), ProtocolError> {
    let age = now_ms.saturating_sub(timestamp);
    if age > ENVELOPE_FRESHNESS_WINDOW_MS {
        return Err(ProtocolError::Verification(format!(
            "stale envelope ({age} ms old, window {ENVELOPE_FRESHNESS_WINDOW_MS} ms)"
        )));
    }
    let ahead = timestamp.saturating_sub(now_ms);
    if ahead > ENVELOPE_MAX_FUTURE_SKEW_MS {
        return Err(ProtocolError::Verification(format!(
            "envelope timestamp {ahead} ms in the future (max {ENVELOPE_MAX_FUTURE_SKEW_MS} ms)"
        )));
    }
    Ok(())
}

/// Deserialize the decrypted payload into a `MessagePayload` enum.
pub fn parse_payload(decrypted: &[u8]) -> Result<MessagePayload, ProtocolError> {
    serde_json::from_slice(decrypted)
        .map_err(|e| ProtocolError::Deserialization(format!("payload parse failed: {e}")))
}

/// The form an envelope's payload arrived in: a plain payload is its JSON,
/// a Signal ciphertext is binary and never parses as JSON.
#[must_use]
pub fn received_sealing(envelope_payload: &[u8]) -> Sealing {
    if serde_json::from_slice::<serde_json::Value>(envelope_payload).is_ok() {
        Sealing::Plain
    } else {
        Sealing::Session
    }
}

/// Parse an opened payload (decrypted when it arrived `Session`-sealed) and
/// require that it arrived sealed as its type demands, so a session
/// payload is never accepted in plain, nor a plain one through the session.
pub fn parse_sealed_payload(
    opened: &[u8],
    received: Sealing,
) -> Result<MessagePayload, ProtocolError> {
    let payload = parse_payload(opened)?;
    let required = payload.sealing();
    if required != received {
        return Err(ProtocolError::Verification(format!(
            "payload requires {required:?} sealing, arrived {received:?}"
        )));
    }
    Ok(payload)
}

/// Parse an incoming envelope and check it is signed by its sender for
/// `my_pubkey` and fresh at `now_ms`. Duplicate suppression
/// ([`crate::messaging::replay::ReplayGuard`]) and decryption happen in
/// the service layer.
pub fn process_incoming(
    raw: &[u8],
    my_pubkey: &[u8; 32],
    now_ms: u64,
) -> Result<MessageEnvelope, ProtocolError> {
    let envelope = parse_envelope(raw)?;
    verify_envelope(&envelope, my_pubkey)?;
    check_freshness(envelope.timestamp, now_ms)?;
    Ok(envelope)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;
    use crate::messaging::sender::build_envelope;

    const BOB: [u8; 32] = [7u8; 32];
    const CAROL: [u8; 32] = [9u8; 32];
    const NOW: u64 = 1_800_000_000_000;

    fn alice() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    fn to_bob(payload: &[u8]) -> MessageEnvelope {
        build_envelope(&alice(), &BOB, NOW, vec![1u8; 16], payload.to_vec())
    }

    #[test]
    fn round_trip() {
        let raw = capnp_codec::message::encode_envelope(&to_bob(b"{}"));
        assert!(process_incoming(&raw, &BOB, NOW + 1_000).is_ok());
    }

    /// A captured `Unfriended` replayed to a third identity fails there.
    #[test]
    fn wrong_recipient_rejected() {
        let env = to_bob(br#"{"type":"Unfriended"}"#);
        assert!(verify_envelope(&env, &BOB).is_ok());
        assert!(verify_envelope(&env, &CAROL).is_err());
    }

    /// The pre-v2 layout (`ts || nonce || payload`, no domain or recipient)
    /// no longer verifies.
    #[test]
    fn v1_signature_rejected() {
        let mut env = to_bob(b"{}");
        let mut v1 = env.timestamp.to_le_bytes().to_vec();
        v1.extend_from_slice(&env.nonce);
        v1.extend_from_slice(&env.payload);
        env.signature = alice().sign(&v1).to_bytes().to_vec();
        assert!(verify_envelope(&env, &BOB).is_err());
    }

    /// Moving bytes across the nonce/payload boundary changes the length
    /// prefixes, so it breaks the signature even though the concatenation
    /// is the same.
    #[test]
    fn shifted_nonce_payload_boundary_rejected() {
        let mut env = to_bob(b"{}");
        let first = env.payload.remove(0);
        env.nonce.push(first);
        assert!(verify_envelope(&env, &BOB).is_err());
    }

    #[test]
    fn stale_and_future_timestamps_rejected() {
        assert!(check_freshness(NOW, NOW + ENVELOPE_FRESHNESS_WINDOW_MS).is_ok());
        assert!(check_freshness(NOW, NOW + ENVELOPE_FRESHNESS_WINDOW_MS + 1).is_err());
        assert!(check_freshness(NOW + ENVELOPE_MAX_FUTURE_SKEW_MS, NOW).is_ok());
        assert!(check_freshness(NOW + ENVELOPE_MAX_FUTURE_SKEW_MS + 1, NOW).is_err());
        let raw = capnp_codec::message::encode_envelope(&to_bob(b"{}"));
        assert!(process_incoming(&raw, &BOB, NOW + ENVELOPE_FRESHNESS_WINDOW_MS + 1).is_err());
    }

    /// A session payload that arrives in plain is dropped, and a plain one
    /// cannot ride the session.
    #[test]
    fn sealing_must_match_payload_type() {
        let dm = serde_json::to_vec(&MessagePayload::DirectMessage {
            body: "hi".into(),
            reply_to: None,
        })
        .unwrap();
        assert_eq!(received_sealing(&dm), Sealing::Plain);
        assert!(parse_sealed_payload(&dm, Sealing::Plain).is_err());
        assert!(parse_sealed_payload(&dm, Sealing::Session).is_ok());

        let unfriended = serde_json::to_vec(&MessagePayload::Unfriended).unwrap();
        assert!(parse_sealed_payload(&unfriended, Sealing::Plain).is_ok());
        assert!(parse_sealed_payload(&unfriended, Sealing::Session).is_err());

        assert_eq!(received_sealing(&[0x8a, 0x01, 0xff]), Sealing::Session);
    }
}
