use super::*;
use rekindle_codec::community::envelope::CommunityEnvelope;

#[test]
fn accepts_matching_ciphertext() {
    let ciphertext = b"encrypted message payload from SMPL write";
    let hash = blake3_hex(ciphertext);
    assert!(verify_content_hash(ciphertext, &hash));
}

#[test]
fn rejects_tampered_ciphertext() {
    let original = b"encrypted message payload from SMPL write";
    let hash = blake3_hex(original);
    let tampered = b"tampered message payload from evil node";
    assert!(!verify_content_hash(tampered, &hash));
}

#[test]
fn rejects_single_bit_flip() {
    let original = b"encrypted message payload";
    let hash = blake3_hex(original);
    let mut flipped = original.to_vec();
    flipped[0] ^= 1;
    assert!(!verify_content_hash(&flipped, &hash));
}

#[test]
fn empty_ciphertext_has_deterministic_hash() {
    let hash1 = blake3_hex(b"");
    let hash2 = blake3_hex(b"");
    assert_eq!(hash1, hash2);
    assert!(verify_content_hash(b"", &hash1));
}

#[test]
fn rejects_tampered_value_for_watch_relay_hash() {
    let value = b"value-the-observer-saw";
    let relay = CommunityEnvelope::WatchRelay {
        record_key: "VLD0:abc".into(),
        subkey: 4,
        content_hash: blake3_hex(value),
        observer_pseudonym: "observer_01".into(),
    };

    let tampered = b"value-from-attacker";
    let CommunityEnvelope::WatchRelay {
        content_hash: expected_hash,
        ..
    } = relay
    else {
        unreachable!()
    };

    assert!(!verify_content_hash(tampered, &expected_hash));
}
