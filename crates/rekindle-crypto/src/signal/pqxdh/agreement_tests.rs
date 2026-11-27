use super::*;
use crate::signal::test_support::{BobKeys, TestRandom};

fn run(bob: &BobKeys, one_time: bool) -> (InitiatorAgreement, SessionKeys, AssociatedData) {
    let alice = Identity::generate();
    let bundle = bob.bundle(one_time);
    let agreed = initiate(&alice, &bundle, &TestRandom::new(1)).unwrap();
    let (keys, ad) = respond(
        &bob.responder_keys(&agreed),
        &alice.public_key(),
        &agreed.base_key,
        &agreed.kem_ciphertext,
    )
    .unwrap();
    (agreed, keys, ad)
}

#[test]
fn both_sides_derive_both_keys_and_the_ad() {
    let bob = BobKeys::new();
    for one_time in [true, false] {
        let (agreed, keys, ad) = run(&bob, one_time);
        assert_eq!(*agreed.keys.sk_ec, *keys.sk_ec);
        assert_eq!(*agreed.keys.sk_scka, *keys.sk_scka);
        assert_ne!(*keys.sk_ec, *keys.sk_scka);
        assert_eq!(agreed.ad, ad);
        assert_eq!(agreed.one_time_prekey_id.is_some(), one_time);
        assert_eq!(matches!(agreed.pq_prekey, PqPrekeyId::OneTime(_)), one_time);
    }
}

#[test]
fn the_ad_names_the_initiator_first() {
    let bob = BobKeys::new();
    let (_, _, ad) = run(&bob, false);
    assert_eq!(
        &ad[ENCODED_EC_LEN..],
        &encode_identity(&bob.identity.public_key())
    );
}

#[test]
fn a_wrong_kem_secret_derives_other_keys() {
    let bob = BobKeys::new();
    let alice = Identity::generate();
    let agreed = initiate(&alice, &bob.bundle(true), &TestRandom::new(2)).unwrap();
    let mut keys = bob.responder_keys(&agreed);
    keys.pq_prekey = &bob.pq_last_resort;
    let (derived, _) = respond(
        &keys,
        &alice.public_key(),
        &agreed.base_key,
        &agreed.kem_ciphertext,
    )
    .unwrap();
    assert_ne!(*derived.sk_ec, *agreed.keys.sk_ec);
}

#[test]
fn an_unsigned_one_time_key_is_refused() {
    let bob = BobKeys::new();
    let mut bundle = bob.bundle(true);
    bundle.pqpk_ot_signature = None;
    let result = initiate(&Identity::generate(), &bundle, &TestRandom::new(3));
    assert!(matches!(result, Err(PqxdhError::IncompleteBundle(_))));
    let mut bundle = bob.bundle(true);
    bundle.one_time_prekey_signature = None;
    let result = initiate(&Identity::generate(), &bundle, &TestRandom::new(3));
    assert!(matches!(result, Err(PqxdhError::IncompleteBundle(_))));
}

/// A one-time curve key under a signature for another id is refused
/// before any DH runs.
#[test]
fn a_one_time_key_signed_for_another_id_is_refused() {
    let bob = BobKeys::new();
    let mut bundle = bob.bundle(true);
    bundle.one_time_prekey_id = bundle.one_time_prekey_id.map(|id| id + 1);
    let result = initiate(&Identity::generate(), &bundle, &TestRandom::new(5));
    assert!(matches!(result, Err(PqxdhError::SignatureVerify(_))));
}

/// The init names the bundle's own ids, not fixed ones.
#[test]
fn the_agreement_names_the_bundle_ids() {
    let bob = BobKeys::new();
    let alice = Identity::generate();
    let agreed = initiate(&alice, &bob.bundle(false), &TestRandom::new(6)).unwrap();
    assert_eq!(agreed.pq_prekey, PqPrekeyId::LastResort(bob.pq_lr_id));
}

#[test]
fn a_low_order_base_key_is_refused() {
    let bob = BobKeys::new();
    let alice = Identity::generate();
    let agreed = initiate(&alice, &bob.bundle(false), &TestRandom::new(4)).unwrap();
    let result = respond(
        &bob.responder_keys(&agreed),
        &alice.public_key(),
        &[0u8; 32],
        &agreed.kem_ciphertext,
    );
    assert!(matches!(result, Err(PqxdhError::NonContributory)));
}
