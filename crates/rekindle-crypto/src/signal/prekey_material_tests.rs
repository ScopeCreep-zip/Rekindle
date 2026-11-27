use super::*;
use crate::signal::prekey_record::{mint_head, mint_slot};
use crate::signal::test_support::TestRandom;
use crate::signal::{PreKeyBundle, SessionRecord};

/// A record minted here runs a session whose responder keys come
/// back from the stored bytes.
#[test]
fn a_minted_record_runs_a_session_from_stored_secrets() {
    let bob = Identity::generate();
    let alice = Identity::generate();
    let rng = TestRandom::new(1);
    let head = mint_head(&bob, &rng, 5, 6, 1_000);
    let slot = mint_slot(&bob, &rng, 1, 7, 9);
    let bundle = PreKeyBundle::from_record(
        &bob.public_key_bytes(),
        &head.record,
        Some(&slot.record),
        1_000,
    )
    .unwrap();
    let (_, init) = SessionRecord::new(bob.public_key_bytes())
        .initiate(&alice, &bundle, b"hi", b"", 1_000, &rng)
        .unwrap();
    assert_eq!(init.one_time_prekey_id, Some(7));
    assert_eq!(init.signed_prekey_id, 5);

    let secrets = ResponderSecrets::from_bytes(
        &head.spk.secret,
        Some(&slot.opk.secret),
        &slot.pq_opk.secret,
    )
    .unwrap();
    let (_, first) = SessionRecord::new(alice.public_key_bytes())
        .insert_from_init(&init, &secrets.keys(&bob), b"", 1_000, &rng)
        .unwrap();
    assert_eq!(first, b"hi");
}

#[test]
fn short_secrets_are_refused() {
    assert!(
        ResponderSecrets::from_bytes(&[1; 31], None, &mint_pq(&TestRandom::new(3)).secret).is_err()
    );
    assert!(ResponderSecrets::from_bytes(&[1; 32], None, &[0; 10]).is_err());
}
