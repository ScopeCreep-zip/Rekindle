use super::*;

#[test]
fn derived_key_matches_on_both_sides() {
    let (alice_sk, alice_pub) = fresh_keypair();
    let (bob_sk, bob_pub) = fresh_keypair();

    let alice_key = derive_call_key(&alice_sk, &bob_pub, "call-abc").unwrap();
    let bob_key = derive_call_key(&bob_sk, &alice_pub, "call-abc").unwrap();
    assert_eq!(alice_key, bob_key);
}

#[test]
fn different_call_ids_produce_different_keys() {
    let (alice_sk, _) = fresh_keypair();
    let (_, bob_pub) = fresh_keypair();

    let k1 = derive_call_key(&alice_sk, &bob_pub, "call-1").unwrap();
    let k2 = derive_call_key(&alice_sk, &bob_pub, "call-2").unwrap();
    assert_ne!(k1, k2);
}

#[test]
fn rejects_wrong_length_pubkey() {
    let (alice_sk, _) = fresh_keypair();
    let result = derive_call_key(&alice_sk, &[0u8; 16], "x");
    assert!(matches!(result, Err(CallKeyError::InvalidPublicKey(16))));
}
