use super::*;
use ed25519_dalek::{Signer, SigningKey};

#[test]
fn a_sealed_key_opens_only_with_its_session_key_and_binding() {
    let (bob, carol) = (SessionKey::fresh(), SessionKey::fresh());
    let secret = super::super::fresh();
    let sealed = seal(&bob.public(), b"aad", &secret).unwrap();
    assert_eq!(*bob.open(b"aad", &sealed).unwrap(), *secret);
    assert!(carol.open(b"aad", &sealed).is_err());
    assert!(bob.open(b"other", &sealed).is_err());
}

/// A key sealed before the seal moved onto `crate::hpke` (session
/// secret `[7; 32]`, AAD `seal_aad("ch", "bob", 5)`, media secret
/// `[0x42; 32]`) still opens: the wire bytes are unchanged.
#[test]
fn a_key_sealed_before_the_wrapper_still_opens() {
    let bob = SessionKey {
        secret: Zeroizing::new([7; 32]),
        public: hpke::public_key(&[7; 32]),
    };
    assert_eq!(
        hex::encode(bob.public()),
        "13be4feaeaf204c7fd3358fc9c00721881d174278128227ec674f37f7fe97b6d"
    );
    let sealed = hex::decode(
        "0d47ccd7909195755c3d1cd06cfaf76d3636ab55053849473f59c50b1477272f\
             755afa0fec2d36039073e86f18cfb9fbe675f8deb57f303024f9ba472f2703a6\
             d16d865012c8ff026a07d2e626a9ab7e",
    )
    .unwrap();
    let aad = super::super::seal_aad("ch", "bob", 5);
    assert_eq!(*bob.open(&aad, &sealed).unwrap(), [0x42; 32]);
}

/// Once the session key is gone nothing recorded opens: no long-term
/// secret reaches a sealed key (RFC 9180 §9.1).
#[test]
fn a_recorded_key_does_not_open_after_the_session_ends() {
    let bob = SessionKey::fresh();
    let sealed = seal(&bob.public(), b"aad", &super::super::fresh()).unwrap();
    drop(bob);
    // A new session key, or any other key, is not the erased one.
    assert!(SessionKey::fresh().open(b"aad", &sealed).is_err());
}

#[test]
fn signatures_bind_signer_and_message() {
    let alice = SigningKey::from_bytes(&[1; 32]);
    let alice_hex = hex::encode(alice.verifying_key().to_bytes());
    let msg = sealed_key_message("ch", "bob", 7, b"sealed");
    let sig = alice.sign(&msg).to_bytes();
    assert!(verify_pseudonym(&alice_hex, &msg, &sig));
    assert!(!verify_pseudonym(
        &alice_hex,
        &sealed_key_message("ch", "bob", 8, b"sealed"),
        &sig
    ));
    assert!(!verify_pseudonym(&"00".repeat(32), &msg, &sig));
    assert!(!verify_pseudonym(&alice_hex, &msg, b"short"));
}
