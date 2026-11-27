use super::*;
use crate::signal::prekey_record::{mint_head, mint_slot};
use crate::signal::test_support::TestRandom;
use crate::Identity;

#[test]
fn a_record_assembles_a_bundle_with_and_without_a_slot() {
    let bob = Identity::generate();
    let rng = TestRandom::new(1);
    let head = mint_head(&bob, &rng, 11, 12, 1_000);
    let slot = mint_slot(&bob, &rng, 1, 21, 22);
    let bundle = PreKeyBundle::from_record(
        &bob.public_key_bytes(),
        &head.record,
        Some(&slot.record),
        1_000,
    )
    .unwrap();
    assert_eq!((bundle.signed_prekey_id, bundle.pq_lr_id), (11, 12));
    assert_eq!(
        (bundle.one_time_prekey_id, bundle.pqpk_ot_id),
        (Some(21), Some(22))
    );
    let bare =
        PreKeyBundle::from_record(&bob.public_key_bytes(), &head.record, None, 1_000).unwrap();
    assert!(bare.one_time_prekey.is_none() && bare.pqpk_ot.is_none());
}

/// The signatures are checked under the pinned identity: a record
/// signed by anyone else is refused, as is a key moved to another id.
#[test]
fn a_record_under_another_identity_or_other_ids_is_refused() {
    let (bob, mallory) = (Identity::generate(), Identity::generate());
    let rng = TestRandom::new(2);
    let head = mint_head(&mallory, &rng, 11, 12, 1_000);
    assert!(PreKeyBundle::from_record(&bob.public_key_bytes(), &head.record, None, 1_000).is_err());

    let head = mint_head(&bob, &rng, 11, 12, 1_000);
    let mut slot = mint_slot(&bob, &rng, 1, 21, 22).record;
    slot.opk_id = 23;
    assert!(
        PreKeyBundle::from_record(&bob.public_key_bytes(), &head.record, Some(&slot), 1_000)
            .is_err()
    );
    let mut moved = head.record.clone();
    moved.spk_id = 13;
    assert!(PreKeyBundle::from_record(&bob.public_key_bytes(), &moved, None, 1_000).is_err());
    let mut redated = head.record.clone();
    redated.issued_at += 1;
    assert!(PreKeyBundle::from_record(&bob.public_key_bytes(), &redated, None, 1_000).is_err());
}

#[test]
fn an_id_outside_u31_is_refused() {
    let bob = Identity::generate();
    let rng = TestRandom::new(3);
    for id in [0, 0x8000_0000] {
        let head = mint_head(&bob, &rng, id, 12, 1_000);
        assert!(
            PreKeyBundle::from_record(&bob.public_key_bytes(), &head.record, None, 1_000).is_err(),
            "id {id}"
        );
    }
}

/// A head dated more than `SIGNED_MAX_AGE_MS` from now, either way, is
/// refused as stale; one at the bound is used.
#[test]
fn a_head_outside_the_age_bound_is_refused() {
    use crate::signal::prekey_record::SIGNED_MAX_AGE_MS;
    let bob = Identity::generate();
    let rng = TestRandom::new(4);
    let at = 1_000_000_000_000;
    let head = mint_head(&bob, &rng, 11, 12, at).record;
    let read = |now: i64| PreKeyBundle::from_record(&bob.public_key_bytes(), &head, None, now);
    assert!(read(at + SIGNED_MAX_AGE_MS).is_ok());
    assert!(read(at - SIGNED_MAX_AGE_MS).is_ok());
    for now in [at + SIGNED_MAX_AGE_MS + 1, at - SIGNED_MAX_AGE_MS - 1] {
        assert!(matches!(read(now), Err(CryptoError::StalePrekeys { .. })));
    }
}
