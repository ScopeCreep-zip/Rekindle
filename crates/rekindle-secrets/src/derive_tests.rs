use super::*;
use ed25519_dalek::{Signer, VerifyingKey};

#[test]
fn pseudonym_deterministic() {
    let secret = [42u8; 32];
    let k1 = derive_community_pseudonym(&secret, "community_abc");
    let k2 = derive_community_pseudonym(&secret, "community_abc");
    assert_eq!(k1.to_bytes(), k2.to_bytes());
}

#[test]
fn pseudonym_unlinkable_across_communities() {
    let secret = [42u8; 32];
    let k1 = derive_community_pseudonym(&secret, "community_abc");
    let k2 = derive_community_pseudonym(&secret, "community_xyz");
    assert_ne!(k1.to_bytes(), k2.to_bytes());
}

#[test]
fn pseudonym_different_secrets() {
    let k1 = derive_community_pseudonym(&[1u8; 32], "c");
    let k2 = derive_community_pseudonym(&[2u8; 32], "c");
    assert_ne!(k1.to_bytes(), k2.to_bytes());
}

#[test]
fn pseudonym_produces_valid_key() {
    let key = derive_community_pseudonym(&[99u8; 32], "test");
    let verifying = VerifyingKey::from(&key);
    let sig = key.sign(b"test message");
    assert!(verifying.verify_strict(b"test message", &sig).is_ok());
}

#[test]
fn pseudonym_x25519_conversion() {
    let key = derive_community_pseudonym(&[77u8; 32], "test");
    let _x25519 = pseudonym_to_x25519(&key);
}

#[test]
fn sign_and_verify() {
    let key = derive_community_pseudonym(&[1u8; 32], "c");
    let sig = sign_with_pseudonym(&key, b"hello");
    let verifying = VerifyingKey::from(&key);
    let signature = ed25519_dalek::Signature::from_bytes(&sig);
    assert!(verifying.verify_strict(b"hello", &signature).is_ok());
}

#[test]
fn file_cache_name_key_is_stable_per_secret_and_unlinkable_across_secrets() {
    let a = derive_file_cache_name_key(&[5u8; 32]);
    assert_eq!(*a, *derive_file_cache_name_key(&[5u8; 32]));
    assert_ne!(*a, *derive_file_cache_name_key(&[6u8; 32]));
    // Not the pseudonym of any community: its own label.
    assert_ne!(*a, derive_community_pseudonym(&[5u8; 32], "").to_bytes());
}

/// The label is HKDF `info` under no salt: the derivation is exactly
/// HKDF-SHA256(salt = none, ikm = secret, info = FILE_CACHE_NAMES).
#[test]
fn file_cache_name_key_puts_its_label_in_info() {
    let secret = [5u8; 32];
    let mut expected = [0u8; 32];
    Hkdf::<Sha256>::new(None, &secret)
        .expand(domains::FILE_CACHE_NAMES.as_bytes(), &mut expected)
        .unwrap();
    assert_eq!(*derive_file_cache_name_key(&secret), expected);
}

#[test]
fn slot_keypair_deterministic() {
    let seed = [42u8; 32];
    let k1 = derive_slot_keypair(&seed, 0).unwrap();
    let k2 = derive_slot_keypair(&seed, 0).unwrap();
    assert_eq!(k1.to_bytes(), k2.to_bytes());
}

#[test]
fn slot_keypair_different_slots() {
    let seed = [42u8; 32];
    let k1 = derive_slot_keypair(&seed, 0).unwrap();
    let k2 = derive_slot_keypair(&seed, 1).unwrap();
    assert_ne!(k1.to_bytes(), k2.to_bytes());
}

#[test]
fn slot_keypair_different_seeds() {
    let k1 = derive_slot_keypair(&[1u8; 32], 0).unwrap();
    let k2 = derive_slot_keypair(&[2u8; 32], 0).unwrap();
    assert_ne!(k1.to_bytes(), k2.to_bytes());
}

#[test]
fn slot_keypair_valid_range() {
    let seed = [1u8; 32];
    // Should work for all 255 SMPL slots
    for i in 0..255u32 {
        assert!(derive_slot_keypair(&seed, i).is_ok());
    }
}
