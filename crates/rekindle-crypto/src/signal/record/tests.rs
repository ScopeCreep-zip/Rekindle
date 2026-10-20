//! `SessionRecord` cases, including the `signal/test_stores.rs` cases plan
//! E2.3 deletes, re-expressed on the pure API: bundle handouts, crossing
//! first messages, a changed identity refused, and one-time keys consumed
//! only after the first decrypt.

use rand::seq::SliceRandom;

use super::{PreKeyInit, SessionRecord, ARCHIVED_STATES_MAX_LENGTH};
use crate::error::{CoreCryptoError, CryptoError};
use crate::signal::test_support::{BobKeys, TestRandom};
use crate::signal::RatchetMessage;
use crate::Identity;

const AD: &[u8] = b"typeId";

fn seed(base: u64, index: usize) -> u64 {
    base + u64::try_from(index).expect("a test index fits u64")
}

/// `alice` starts a session with `bob` from `bundle`; `bob` accepts it.
fn establish(
    alice: &Identity,
    bob: &BobKeys,
    bundle: &crate::signal::PreKeyBundle,
) -> (SessionRecord, SessionRecord, PreKeyInit) {
    let rng = TestRandom::new(1);
    let (alice_record, init) = SessionRecord::new(bob.identity.public_key_bytes())
        .initiate(alice, bundle, b"hello", AD, &rng)
        .unwrap();
    let (bob_record, first) =
        accept(&SessionRecord::new(alice.public_key_bytes()), bob, &init).unwrap();
    assert_eq!(first, b"hello");
    (alice_record, bob_record, init)
}

fn accept(
    record: &SessionRecord,
    bob: &BobKeys,
    init: &PreKeyInit,
) -> Result<(SessionRecord, Vec<u8>), CryptoError> {
    record.insert_from_init(
        init,
        &bob.keys_for(init.one_time_prekey_id, init.pq_prekey),
        AD,
        &TestRandom::for_message(&init.message.ciphertext),
    )
}

fn send(record: &mut SessionRecord, plaintext: &[u8]) -> RatchetMessage {
    let (next, message) = record.encrypt(plaintext, AD).unwrap();
    *record = next;
    message
}

fn receive(record: &mut SessionRecord, message: &RatchetMessage) -> Result<Vec<u8>, CryptoError> {
    let (next, plaintext) =
        record.decrypt(message, AD, &TestRandom::for_message(&message.ciphertext))?;
    *record = next;
    Ok(plaintext)
}

fn round_trip(a: &mut SessionRecord, b: &mut SessionRecord) {
    let ping = send(a, b"ping");
    assert_eq!(receive(b, &ping).unwrap(), b"ping");
    let pong = send(b, b"pong");
    assert_eq!(receive(a, &pong).unwrap(), b"pong");
}

#[test]
fn a_session_runs_with_and_without_one_time_keys() {
    let bob = BobKeys::new();
    for one_time in [true, false] {
        let alice = Identity::generate();
        let (mut a, mut b, init) = establish(&alice, &bob, &bob.bundle(one_time));
        assert_eq!(init.one_time_prekey_id.is_some(), one_time);
        for _ in 0..3 {
            round_trip(&mut a, &mut b);
        }
    }
}

#[test]
fn the_stored_form_round_trips_and_rejects_corruption() {
    let bob = BobKeys::new();
    let alice = Identity::generate();
    let (mut a, b, _) = establish(&alice, &bob, &bob.bundle(true));
    let skipped = send(&mut a, b"skipped");
    let later = send(&mut a, b"later");

    let mut restored = SessionRecord::from_bytes(&b.to_bytes()).unwrap();
    assert_eq!(restored.peer_identity(), b.peer_identity());
    assert_eq!(receive(&mut restored, &later).unwrap(), b"later");
    let mut again = SessionRecord::from_bytes(&restored.to_bytes()).unwrap();
    assert_eq!(receive(&mut again, &skipped).unwrap(), b"skipped");

    let bytes = b.to_bytes();
    assert!(SessionRecord::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(SessionRecord::from_bytes(&trailing).is_err());
    let mut format = bytes.to_vec();
    format[0] = 9;
    assert!(SessionRecord::from_bytes(&format).is_err());
    let empty = SessionRecord::new([5; 32]);
    let back = SessionRecord::from_bytes(&empty.to_bytes()).unwrap();
    assert!(!back.has_active());
}

#[test]
fn encrypt_without_a_session_fails() {
    let record = SessionRecord::new([4; 32]);
    assert!(matches!(
        record.encrypt(b"x", AD),
        Err(CryptoError::NoSession(_))
    ));
}

/// Every handed-out bundle completes once, in whatever order its init
/// arrives; a replayed init is a no-op.
#[test]
fn bundle_handouts_complete_independently() {
    let peers = 12;
    let bob = BobKeys::with_one_time(peers);
    let alices: Vec<Identity> = (0..peers).map(|_| Identity::generate()).collect();
    let inits: Vec<(SessionRecord, PreKeyInit)> = alices
        .iter()
        .enumerate()
        .map(|(i, alice)| {
            SessionRecord::new(bob.identity.public_key_bytes())
                .initiate(
                    alice,
                    &bob.handout(i),
                    b"hi",
                    AD,
                    &TestRandom::new(seed(10, i)),
                )
                .unwrap()
        })
        .collect();
    let mut order: Vec<usize> = (0..alices.len()).collect();
    order.shuffle(&mut rand::rngs::OsRng);
    for &i in &order {
        let (mut a, init) = inits[i].clone();
        let empty = SessionRecord::new(alices[i].public_key_bytes());
        let (mut b, first) = accept(&empty, &bob, &init).unwrap();
        assert_eq!(first, b"hi");
        round_trip(&mut a, &mut b);
        assert!(matches!(
            accept(&b, &bob, &init),
            Err(CryptoError::DuplicateMessage)
        ));
    }
}

/// Both sides initiate at once; each accepts the other's init. Every
/// message decrypts, and the two records settle on one session.
#[test]
fn crossing_first_messages_both_decrypt_and_converge() {
    let a_keys = BobKeys::new();
    let b_keys = BobKeys::new();
    let rng = TestRandom::new(5);
    let (a, a_init) = SessionRecord::new(b_keys.identity.public_key_bytes())
        .initiate(&a_keys.identity, &b_keys.bundle(true), b"from a", AD, &rng)
        .unwrap();
    let (b, b_init) = SessionRecord::new(a_keys.identity.public_key_bytes())
        .initiate(&b_keys.identity, &a_keys.bundle(true), b"from b", AD, &rng)
        .unwrap();
    let (mut a, from_b) = accept(&a, &a_keys, &b_init).unwrap();
    let (mut b, from_a) = accept(&b, &b_keys, &a_init).unwrap();
    assert_eq!(
        (from_a.as_slice(), from_b.as_slice()),
        (&b"from a"[..], &b"from b"[..])
    );
    assert_eq!((a.inactive.len(), b.inactive.len()), (1, 1));

    // Both send before either hears the other's session choice.
    let m_a = send(&mut a, b"a1");
    let m_b = send(&mut b, b"b1");
    assert_eq!(receive(&mut b, &m_a).unwrap(), b"a1");
    assert_eq!(receive(&mut a, &m_b).unwrap(), b"b1");
    for _ in 0..3 {
        round_trip(&mut a, &mut b);
        round_trip(&mut b, &mut a);
    }
    assert_eq!(
        a.active.as_ref().unwrap().base_key(),
        b.active.as_ref().unwrap().base_key()
    );
}

/// A bundle or init under another identity key is refused on both sides
/// and changes nothing (identity == address).
#[test]
fn a_changed_identity_is_refused_on_both_sides() {
    let bob = BobKeys::new();
    let alice = Identity::generate();
    let (a, b, _) = establish(&alice, &bob, &bob.bundle(true));
    let mallory = BobKeys::new();

    let forged = mallory.bundle(true);
    let Err(err) = a.initiate(&alice, &forged, b"x", AD, &TestRandom::new(6)) else {
        panic!("a bundle under another key must be refused");
    };
    assert!(
        matches!(err, CryptoError::IdentityAddressMismatch { .. }),
        "{err}"
    );

    let (_, init) = SessionRecord::new(bob.identity.public_key_bytes())
        .initiate(
            &mallory.identity,
            &bob.bundle(false),
            b"x",
            AD,
            &TestRandom::new(7),
        )
        .unwrap();
    let Err(err) = accept(&b, &bob, &init) else {
        panic!("an init under another key must be refused");
    };
    assert!(
        matches!(err, CryptoError::IdentityAddressMismatch { .. }),
        "{err}"
    );

    let (mut a, mut b) = (a, b);
    round_trip(&mut a, &mut b);
}

fn is_own_identity(err: &CryptoError) -> bool {
    matches!(err, CryptoError::Core(CoreCryptoError::InvalidKey(msg)) if msg.contains("our own identity"))
}

/// A bundle or an init that names our own identity is refused on both
/// sides with the own-identity error, before any agreement.
#[test]
fn a_session_with_ourselves_is_refused() {
    let bob = BobKeys::new();
    let ours = SessionRecord::new(bob.identity.public_key_bytes());
    let Err(err) = ours.initiate(
        &bob.identity,
        &bob.bundle(false),
        b"x",
        AD,
        &TestRandom::new(8),
    ) else {
        panic!("a bundle under our own key must be refused");
    };
    assert!(is_own_identity(&err), "{err}");

    let alice = Identity::generate();
    let (_, mut init) = SessionRecord::new(bob.identity.public_key_bytes())
        .initiate(&alice, &bob.bundle(false), b"x", AD, &TestRandom::new(9))
        .unwrap();
    init.identity_key = bob.identity.public_key_bytes();
    let Err(err) = accept(&ours, &bob, &init) else {
        panic!("an init under our own key must be refused");
    };
    assert!(is_own_identity(&err), "{err}");
}

/// The caller consumes the one-time keys only on `Ok`: an init whose first
/// message fails yields nothing, so the same keys still accept the real
/// init; once it is in, the init is a no-op.
#[test]
fn one_time_keys_are_consumed_only_after_the_first_decrypt() {
    let bob = BobKeys::new();
    let alice = Identity::generate();
    let (_, init) = SessionRecord::new(bob.identity.public_key_bytes())
        .initiate(&alice, &bob.bundle(true), b"first", AD, &TestRandom::new(9))
        .unwrap();
    assert!(init.one_time_prekey_id.is_some());
    let empty = SessionRecord::new(alice.public_key_bytes());

    let mut tampered = init.clone();
    tampered.message.ciphertext[0] ^= 1;
    assert!(accept(&empty, &bob, &tampered).is_err());

    let no_opk = bob.keys_for(None, init.pq_prekey);
    assert!(empty
        .insert_from_init(&init, &no_opk, AD, &TestRandom::new(2))
        .is_err());

    let (b, first) = accept(&empty, &bob, &init).unwrap();
    assert_eq!(first, b"first");
    assert!(matches!(
        accept(&b, &bob, &init),
        Err(CryptoError::DuplicateMessage)
    ));
}

/// Re-initiations archive the old sessions, at most the bound, and a
/// message that only an archived session decrypts promotes it.
#[test]
fn inactive_sessions_are_bounded_and_promoted_on_use() {
    let bob = BobKeys::new();
    let alice = Identity::generate();
    let (mut a, mut b, _) = establish(&alice, &bob, &bob.bundle(false));
    let old_session = send(&mut a, b"old");
    let old_base = *b.active.as_ref().unwrap().base_key();

    let (mut a2, init) = a
        .initiate(&alice, &bob.bundle(false), b"new", AD, &TestRandom::new(20))
        .unwrap();
    let (next, _) = accept(&b, &bob, &init).unwrap();
    b = next;
    assert_eq!(receive(&mut b, &old_session).unwrap(), b"old");
    assert_eq!(*b.active.as_ref().unwrap().base_key(), old_base);
    round_trip(&mut a2, &mut b);
    assert_eq!(*b.active.as_ref().unwrap().base_key(), init.base_key);

    for i in 0..ARCHIVED_STATES_MAX_LENGTH + 3 {
        let (next_a, init) = a2
            .initiate(
                &alice,
                &bob.bundle(false),
                b"again",
                AD,
                &TestRandom::new(seed(30, i)),
            )
            .unwrap();
        a2 = next_a;
        b = accept(&b, &bob, &init).unwrap().0;
    }
    assert_eq!(b.inactive.len(), ARCHIVED_STATES_MAX_LENGTH);
    assert_eq!(a2.inactive.len(), ARCHIVED_STATES_MAX_LENGTH);
    let b = SessionRecord::from_bytes(&b.to_bytes()).unwrap();
    assert_eq!(b.inactive.len(), ARCHIVED_STATES_MAX_LENGTH);
}

/// B6: only our unanswered init is pending, and discarding it restores
/// the session it replaced.
#[test]
fn only_an_unanswered_init_is_pending_and_it_can_be_discarded() {
    let (alice, bob) = (Identity::generate(), BobKeys::new());
    let (mut alice_record, mut bob_record, init) = establish(&alice, &bob, &bob.bundle(true));
    assert_eq!(alice_record.pending_init(), Some(&init.base_key));
    assert_eq!(bob_record.pending_init(), None);

    // Bob answers: the init is no longer pending.
    let pong = send(&mut bob_record, b"pong");
    receive(&mut alice_record, &pong).unwrap();
    assert_eq!(alice_record.pending_init(), None);
    assert!(alice_record.without_pending(&init.base_key).is_none());

    // A second init replaces the answered session; discarding it brings
    // that session back.
    let rng = TestRandom::new(9);
    let (retry, second) = alice_record
        .initiate(&alice, &bob.bundle(false), b"again", AD, &rng)
        .unwrap();
    assert!(retry.without_pending(&[0; 32]).is_none());
    let mut restored = retry.without_pending(&second.base_key).unwrap();
    assert_eq!(restored.pending_init(), None);
    round_trip(&mut restored, &mut bob_record);
}
