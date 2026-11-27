use super::*;

#[test]
fn round_trip_encapsulate_decapsulate() {
    let (sk, pk) = MlKemSecret::generate();
    let out = pk.encapsulate();
    let ct =
        ml_kem_ciphertext_from_bytes(&out.ciphertext).expect("ciphertext is exactly 1088 bytes");
    let recovered = sk.decapsulate(&ct);
    assert_eq!(*out.shared_secret, *recovered, "shared secret must match");
}

#[test]
fn public_key_byte_roundtrip() {
    let (_sk, pk) = MlKemSecret::generate();
    let bytes = pk.as_bytes().to_vec();
    assert_eq!(bytes.len(), ML_KEM_PUBLIC_KEY_BYTES);
    let restored = MlKemPublic::from_bytes(&bytes).expect("valid length");
    assert_eq!(restored.as_bytes(), bytes.as_slice());
}

#[test]
fn wrong_secret_returns_different_shared_secret() {
    // ML-KEM-768 implicit rejection: decapsulating with the wrong
    // secret returns a deterministic-but-different shared secret
    // (NOT an error). This is by design for IND-CCA2 security.
    let (sk_alice, pk_alice) = MlKemSecret::generate();
    let (sk_bob, _pk_bob) = MlKemSecret::generate();
    let out = pk_alice.encapsulate();
    let ct = ml_kem_ciphertext_from_bytes(&out.ciphertext).unwrap();
    let alice_view = sk_alice.decapsulate(&ct);
    let bob_view = sk_bob.decapsulate(&ct);
    assert_eq!(*out.shared_secret, *alice_view);
    assert_ne!(*out.shared_secret, *bob_view);
}

#[test]
fn from_bytes_rejects_wrong_length() {
    assert!(MlKemPublic::from_bytes(&[0u8; 1183]).is_none());
    assert!(MlKemPublic::from_bytes(&[0u8; 1185]).is_none());
    assert!(ml_kem_ciphertext_from_bytes(&[0u8; 1087]).is_none());
    assert!(ml_kem_ciphertext_from_bytes(&[0u8; 1089]).is_none());
}

/// FIPS 203 §7.2: an encapsulation key whose `t` holds a coefficient
/// of q (3329) or more is refused; an all-zero `t` (every coefficient
/// reduced) passes the check.
#[test]
fn from_bytes_refuses_an_unreduced_encapsulation_key() {
    let mut key = [0u8; ML_KEM_PUBLIC_KEY_BYTES];
    assert!(MlKemPublic::from_bytes(&key).is_some());
    // The first 12-bit coefficient = 0xFFF = 4095 ≥ q.
    key[0] = 0xFF;
    key[1] = 0x0F;
    assert!(MlKemPublic::from_bytes(&key).is_none());
    // 3329 = 0xD01, exactly q: not reduced.
    key[0] = 0x01;
    key[1] = 0x0D;
    assert!(MlKemPublic::from_bytes(&key).is_none());
    // 3328 = q − 1: reduced.
    key[0] = 0x00;
    assert!(MlKemPublic::from_bytes(&key).is_some());
    // A generated key passes.
    let (_, pk) = MlKemSecret::generate();
    assert!(MlKemPublic::from_bytes(pk.as_bytes()).is_some());
}

#[test]
fn distinct_keypairs_have_distinct_public_keys() {
    // Sanity: OsRng-driven keypair generation produces unique keys.
    let (_, a) = MlKemSecret::generate();
    let (_, b) = MlKemSecret::generate();
    assert_ne!(a.as_bytes(), b.as_bytes());
}

#[test]
fn public_extracted_from_secret_matches_paired_public() {
    let (sk, pk) = MlKemSecret::generate();
    let extracted = sk.public();
    assert_eq!(extracted.as_bytes(), pk.as_bytes());
}

#[test]
fn secret_byte_roundtrip_preserves_public() {
    let (sk, pk) = MlKemSecret::generate();
    let sk_bytes = sk.as_secret_bytes().to_vec();
    let restored = MlKemSecret::from_secret_bytes(&sk_bytes).expect("valid length");
    assert_eq!(restored.public().as_bytes(), pk.as_bytes());
}

#[test]
fn drop_zeroizes_the_private_key_bytes() {
    // Exercises exactly the logic `Drop::drop` runs, without reading
    // memory after deallocation (undefined behavior) to prove it.
    let (mut sk, _pk) = MlKemSecret::generate();
    assert!(
        sk.as_secret_bytes().iter().any(|&b| b != 0),
        "freshly generated key should not already be all-zero"
    );
    sk.zeroize_in_place();
    assert!(
        sk.as_secret_bytes().iter().all(|&b| b == 0),
        "private key bytes must be zero after the zeroize Drop runs"
    );
}
