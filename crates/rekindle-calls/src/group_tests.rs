use super::*;
use crate::fresh_keypair;

#[test]
fn round_trip_wraps_for_each_participant() {
    let (alice_sk, _alice_pub) = fresh_keypair();
    let (bob_sk, bob_pub) = fresh_keypair();
    let (carol_sk, carol_pub) = fresh_keypair();

    let call_key = generate_call_key();
    let call_id = "group-call-xyz";

    let bob_wrap = wrap_call_key(&alice_sk, &bob_pub, call_id, "bob_pub", &call_key).unwrap();
    let carol_wrap = wrap_call_key(&alice_sk, &carol_pub, call_id, "carol_pub", &call_key).unwrap();

    let alice_x25519_pub = x25519_dalek::PublicKey::from(&alice_sk).to_bytes();
    let bob_unwrapped =
        unwrap_call_key(&bob_sk, &alice_x25519_pub, call_id, "bob_pub", &bob_wrap).unwrap();
    let carol_unwrapped = unwrap_call_key(
        &carol_sk,
        &alice_x25519_pub,
        call_id,
        "carol_pub",
        &carol_wrap,
    )
    .unwrap();

    assert_eq!(bob_unwrapped, call_key);
    assert_eq!(carol_unwrapped, call_key);
}

#[test]
fn other_participant_cannot_unwrap() {
    let (alice_sk, _) = fresh_keypair();
    let (bob_sk, bob_pub) = fresh_keypair();
    let (carol_sk, _) = fresh_keypair();

    let call_key = generate_call_key();
    let call_id = "exclusive";
    let bob_wrap = wrap_call_key(&alice_sk, &bob_pub, call_id, "bob", &call_key).unwrap();

    let alice_x25519_pub = x25519_dalek::PublicKey::from(&alice_sk).to_bytes();
    // Carol cannot decrypt Bob's wrap because she doesn't hold Bob's secret.
    let result = unwrap_call_key(&carol_sk, &alice_x25519_pub, call_id, "bob", &bob_wrap);
    assert!(result.is_err());
    // Bob using Carol's pubkey label also fails (nonce mismatch).
    let result = unwrap_call_key(&bob_sk, &alice_x25519_pub, call_id, "carol", &bob_wrap);
    assert!(result.is_err());
}

#[test]
fn truncated_wrap_rejected() {
    let (alice_sk, _) = fresh_keypair();
    let alice_pub = x25519_dalek::PublicKey::from(&alice_sk).to_bytes();
    let result = unwrap_call_key(&alice_sk, &alice_pub, "x", "y", &[0u8; 10]);
    assert!(matches!(result, Err(GroupKeyError::Truncated(10))));
}
