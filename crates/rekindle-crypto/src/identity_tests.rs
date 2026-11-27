use super::*;

#[test]
fn a_record_key_fingerprint_ignores_the_encryption_key() {
    let opaque = "A".repeat(43);
    let plain = record_key_fingerprint(&format!("VLD0:{opaque}")).unwrap();
    let with_secret = record_key_fingerprint(&format!("VLD0:{opaque}:{}", "B".repeat(43))).unwrap();
    assert_eq!(plain, with_secret);
    assert_eq!(plain, key_fingerprint(&[0u8; 32]));
    assert!(record_key_fingerprint("not a key").is_none());
    assert!(record_key_fingerprint("VLD0:short").is_none());
}

#[test]
fn a_fingerprint_groups_the_key_hex_in_fours() {
    assert_eq!(
        key_fingerprint(&[0xab, 0xcd, 0x01, 0x23, 0xff]),
        "abcd 0123 ff"
    );
}

#[test]
fn generate_and_sign() {
    let identity = Identity::generate();
    let message = b"hello rekindle";

    let signature = identity.sign(message);
    assert!(Identity::verify(&identity.public_key(), message, &signature).is_ok());
}

#[test]
fn roundtrip_secret_bytes() {
    let identity = Identity::generate();
    let bytes = *identity.secret_key_bytes();
    let restored = Identity::from_secret_bytes(&bytes);
    assert_eq!(identity.public_key_bytes(), restored.public_key_bytes());
}

#[test]
fn x25519_derivation() {
    let alice = Identity::generate();
    let bob = Identity::generate();

    let alice_secret = alice.to_x25519_secret();
    let bob_public = bob.to_x25519_public();
    let bob_secret = bob.to_x25519_secret();
    let alice_public = alice.to_x25519_public();

    let shared_a = alice_secret.diffie_hellman(&bob_public);
    let shared_b = bob_secret.diffie_hellman(&alice_public);

    assert_eq!(shared_a.as_bytes(), shared_b.as_bytes());
}

#[test]
fn peer_ed25519_to_x25519_matches_own_derivation() {
    // Verify that peer_ed25519_to_x25519 (from public key bytes) produces
    // the same X25519 public key as to_x25519_public (from secret key).
    let identity = Identity::generate();
    let from_secret = identity.to_x25519_public();
    let from_public = Identity::peer_ed25519_to_x25519(&identity.public_key_bytes()).unwrap();
    assert_eq!(from_secret.as_bytes(), from_public.as_bytes());
}

#[test]
fn peer_x25519_dh_agreement() {
    // Alice uses her secret + peer_ed25519_to_x25519(bob's public key)
    // Bob uses his secret + peer_ed25519_to_x25519(alice's public key)
    // Both should derive the same shared secret.
    let alice = Identity::generate();
    let bob = Identity::generate();

    let alice_secret = alice.to_x25519_secret();
    let bob_x25519_pub = Identity::peer_ed25519_to_x25519(&bob.public_key_bytes()).unwrap();

    let bob_secret = bob.to_x25519_secret();
    let alice_x25519_pub = Identity::peer_ed25519_to_x25519(&alice.public_key_bytes()).unwrap();

    let shared_a = alice_secret.diffie_hellman(&bob_x25519_pub);
    let shared_b = bob_secret.diffie_hellman(&alice_x25519_pub);

    assert_eq!(shared_a.as_bytes(), shared_b.as_bytes());
}
