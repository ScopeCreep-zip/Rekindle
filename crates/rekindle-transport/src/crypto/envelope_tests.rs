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

/// A fixed clock reading for the signer and the verifier.
const NOW_MS: u64 = 1_700_000_000_000;

fn signer_hex() -> String {
    hex::encode(SigningKey::from_bytes(&SECRET).verifying_key().to_bytes())
}

fn signed(payload: &[u8]) -> SignedPayload {
    sign_payload(
        &SECRET,
        &signer_hex(),
        to_bob(TypeId::CommunityLeave),
        payload,
        NOW_MS,
    )
}

#[test]
fn sign_verify_roundtrip() {
    let s = signed(b"test payload");
    assert!(verify_signed_payload(&s, to_bob(TypeId::CommunityLeave), NOW_MS).is_ok());
}

/// A captured frame replayed to a third identity fails there.
#[test]
fn wrong_recipient_rejected() {
    let s = signed(b"leave");
    let to_carol = Addressing {
        recipient: &CAROL,
        type_id: TypeId::CommunityLeave,
    };
    assert!(verify_signed_payload(&s, to_carol, NOW_MS).is_err());
}

/// Signed bytes re-framed under another `TypeId` fail.
#[test]
fn reframed_type_id_rejected() {
    let s = signed(b"payload");
    assert!(verify_signed_payload(&s, to_bob(TypeId::GossipBroadcast), NOW_MS).is_err());
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
    assert!(verify_signed_payload(&s, to_bob(TypeId::GossipBroadcast), NOW_MS).is_err());
}

#[test]
fn stale_timestamp_rejected() {
    let to = to_bob(TypeId::CommunityLeave);
    let timestamp = NOW_MS - ENVELOPE_FRESHNESS_WINDOW_MS - 1_000;
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
    assert!(verify_signed_payload(&s, to, NOW_MS).is_err());
}

#[test]
fn tampered_payload_rejected() {
    let mut s = signed(b"original");
    s.payload = b"tampered".to_vec();
    assert!(verify_signed_payload(&s, to_bob(TypeId::GossipBroadcast), NOW_MS).is_err());
}

#[test]
fn tampered_timestamp_rejected() {
    let mut s = signed(b"original");
    s.timestamp -= 1;
    assert!(verify_signed_payload(&s, to_bob(TypeId::GossipBroadcast), NOW_MS).is_err());
}

#[test]
fn wrong_key_rejected() {
    let mut s = signed(b"test");
    s.sender_key_hex = hex::encode(
        SigningKey::from_bytes(&[99u8; 32])
            .verifying_key()
            .to_bytes(),
    );
    assert!(verify_signed_payload(&s, to_bob(TypeId::GossipBroadcast), NOW_MS).is_err());
}
