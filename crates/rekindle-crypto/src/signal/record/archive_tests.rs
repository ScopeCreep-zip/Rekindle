//! Plan E2.5h, the session archive, tested as Rekindle states it (Sesame
//! §3, Double Ratchet rev. 4 §3.5 and §8.4, PQXDH rev. 3 §3.3–§3.4): the
//! archive bound, in-place decrypts that never promote, the crossing
//! tiebreak, the direction-change breaker, and PreKey wrapping until the
//! peer answers.

use super::tests::receive;
use super::{
    Outgoing, PreKeyInit, SessionRecord, MAX_DIRECTION_CHANGES, MAX_INACTIVE_SESSIONS,
    PREKEY_WRAP_MAX_AGE_MS,
};
use crate::error::CryptoError;
use crate::signal::test_support::{BobKeys, TestRandom};
use crate::signal::RatchetMessage;

const AD: &[u8] = b"typeId";
const NOW: i64 = 1_000;

fn initiate(
    from: &BobKeys,
    to: &BobKeys,
    record: &SessionRecord,
    seed: u64,
) -> (SessionRecord, PreKeyInit) {
    record
        .initiate(
            &from.identity,
            &to.bundle(false),
            b"init",
            AD,
            NOW,
            &TestRandom::new(seed),
        )
        .unwrap()
}

fn accept(record: &SessionRecord, keys: &BobKeys, init: &PreKeyInit) -> SessionRecord {
    let (next, first) = record
        .insert_from_init(
            init,
            &keys.keys_for(init.one_time_prekey_id, init.pq_prekey),
            AD,
            NOW,
            &TestRandom::for_message(&init.message.ciphertext),
        )
        .unwrap();
    assert_eq!(first, b"init");
    next
}

fn send_at(record: &mut SessionRecord, plaintext: &[u8], now_ms: i64) -> Outgoing {
    let (next, outgoing) = record.encrypt(plaintext, AD, now_ms).unwrap();
    *record = next;
    outgoing
}

fn send(record: &mut SessionRecord, plaintext: &[u8]) -> RatchetMessage {
    send_at(record, plaintext, NOW).message().clone()
}

fn active_base(record: &SessionRecord) -> [u8; 32] {
    *record.active.as_ref().unwrap().state.base_key()
}

fn archived_bases(record: &SessionRecord) -> Vec<[u8; 32]> {
    record
        .inactive
        .iter()
        .map(|s| *s.state.base_key())
        .collect()
}

/// Alice initiates, Bob accepts and answers, Alice reads the answer.
fn established() -> (BobKeys, BobKeys, SessionRecord, SessionRecord) {
    let (a, b) = (BobKeys::new(), BobKeys::new());
    let (mut alice, init) = initiate(
        &a,
        &b,
        &SessionRecord::new(b.identity.public_key_bytes()),
        1,
    );
    let mut bob = accept(
        &SessionRecord::new(a.identity.public_key_bytes()),
        &b,
        &init,
    );
    let pong = send(&mut bob, b"pong");
    receive(&mut alice, &pong).unwrap();
    (a, b, alice, bob)
}

/// The archive keeps the newest [`MAX_INACTIVE_SESSIONS`], newest first;
/// the oldest leaves (its keys erased), and a message for it no longer
/// decrypts.
#[test]
fn the_archive_keeps_three_newest_first_and_evicts_the_oldest() {
    let (a, b, mut alice, mut bob) = established();
    let first_base = active_base(&bob);
    let for_the_first = send(&mut alice, b"for the first session");
    let mut bases = vec![first_base];
    for seed in 0..3 {
        let (next, init) = initiate(&a, &b, &alice, 10 + seed);
        alice = next;
        bob = accept(&bob, &b, &init);
        bases.push(init.base_key);
        assert!(
            bases.iter().all(|base| bob.has_base_key(base)),
            "nothing evicted below the bound"
        );
    }
    let mut expected = bases[..3].to_vec();
    expected.reverse();
    assert_eq!(archived_bases(&bob), expected);
    assert_eq!(bob.inactive.len(), MAX_INACTIVE_SESSIONS);

    let (next, init) = initiate(&a, &b, &alice, 20);
    drop(next);
    bob = accept(&bob, &b, &init);
    assert!(
        !bob.has_base_key(&first_base),
        "the first session was evicted"
    );
    assert_eq!(bob.inactive.len(), MAX_INACTIVE_SESSIONS);
    assert!(receive(&mut bob, &for_the_first).is_err());
    let bob = SessionRecord::from_bytes(&bob.to_bytes()).unwrap();
    assert_eq!(bob.inactive.len(), MAX_INACTIVE_SESSIONS);
}

/// A late message on an archived session decrypts there, at the same
/// index, and the active session stays the one it was.
#[test]
fn an_archived_session_decrypts_in_place_and_is_never_promoted() {
    let (a, b, mut alice, mut bob) = established();
    let late = send(&mut alice, b"late");
    let (next, init) = initiate(&a, &b, &alice, 7);
    alice = next;
    bob = accept(&bob, &b, &init);
    let order_before = archived_bases(&bob);
    assert_eq!(receive(&mut bob, &late).unwrap(), b"late");
    assert_eq!(active_base(&bob), init.base_key);
    assert_eq!(archived_bases(&bob), order_before);
    let reply = send(&mut bob, b"reply");
    assert_eq!(receive(&mut alice, &reply).unwrap(), b"reply");
    assert_eq!(active_base(&alice), init.base_key);
}

/// Crossing initiations settle on the higher base key on both sides; the
/// lower one's first message is still delivered, on the archived session.
#[test]
fn crossing_initiations_settle_on_the_higher_base_key() {
    for seeds in [(3, 4), (4, 3), (5, 9), (9, 5)] {
        let (a, b) = (BobKeys::new(), BobKeys::new());
        let (alice, a_init) = initiate(
            &a,
            &b,
            &SessionRecord::new(b.identity.public_key_bytes()),
            seeds.0,
        );
        let (bob, b_init) = initiate(
            &b,
            &a,
            &SessionRecord::new(a.identity.public_key_bytes()),
            seeds.1,
        );
        let higher = a_init.base_key.max(b_init.base_key);
        let mut alice = accept(&alice, &a, &b_init);
        let mut bob = accept(&bob, &b, &a_init);
        assert_eq!((active_base(&alice), active_base(&bob)), (higher, higher));
        let ping = send(&mut alice, b"ping");
        assert_eq!(receive(&mut bob, &ping).unwrap(), b"ping");
        let pong = send(&mut bob, b"pong");
        assert_eq!(receive(&mut alice, &pong).unwrap(), b"pong");
        assert_eq!((active_base(&alice), active_base(&bob)), (higher, higher));
    }
}

/// A peer's initiation goes active over an active session the peer has
/// already acknowledged, whatever its base key.
#[test]
fn an_initiation_replaces_an_acknowledged_session() {
    for seed in 30..34 {
        let (a, b, alice, bob) = established();
        let (_, init) = initiate(&b, &a, &bob, seed);
        let alice = accept(&alice, &a, &init);
        assert_eq!(active_base(&alice), init.base_key);
    }
}

/// An initiation under a changed identity is another address's record,
/// which has no active session and takes it as active; the old identity's
/// record refuses it.
#[test]
fn a_changed_identity_initiates_its_own_record() {
    let (a, _b, alice, _) = established();
    let rotated = BobKeys::new();
    let (_, init) = initiate(
        &rotated,
        &a,
        &SessionRecord::new(a.identity.public_key_bytes()),
        8,
    );
    let keys = a.keys_for(init.one_time_prekey_id, init.pq_prekey);
    assert!(matches!(
        alice.insert_from_init(&init, &keys, AD, NOW, &TestRandom::new(1)),
        Err(CryptoError::IdentityAddressMismatch { .. })
    ));
    let fresh = accept(
        &SessionRecord::new(rotated.identity.public_key_bytes()),
        &a,
        &init,
    );
    assert_eq!(active_base(&fresh), init.base_key);
}

/// Sending on a new session while the peer keeps answering on the old one
/// flips direction each time; past [`MAX_DIRECTION_CHANGES`] the active
/// session is archived and the next send has no session.
#[test]
fn the_breaker_archives_the_active_session_after_too_many_flips() {
    let (a, b, alice, mut bob) = established();
    let (mut alice, _) = initiate(&a, &b, &alice, 40);
    let mut flips = 0;
    while alice.has_active() {
        let old = send(&mut bob, b"on the old session");
        receive(&mut alice, &old).unwrap();
        flips += 1;
        if alice.has_active() {
            send(&mut alice, b"on the new session");
            flips += 1;
        }
        assert!(flips <= usize::from(MAX_DIRECTION_CHANGES) + 2);
    }
    assert_eq!(flips, usize::from(MAX_DIRECTION_CHANGES) + 2);
    assert!(matches!(
        alice.encrypt(b"x", AD, NOW),
        Err(CryptoError::NoSession(_))
    ));
}

/// Until the peer answers, every message is a PreKeyInit with the session's
/// base key, which the peer decrypts on that session with no prekey; one
/// decrypt under it ends the wrapping; past the age bound it is not sent on.
#[test]
fn messages_are_wrapped_until_answered_for_at_most_thirty_days() {
    let (a, b) = (BobKeys::new(), BobKeys::new());
    let (mut alice, init) = initiate(
        &a,
        &b,
        &SessionRecord::new(b.identity.public_key_bytes()),
        2,
    );
    let Outgoing::PreKey(second) = send_at(&mut alice, b"second", NOW + 1) else {
        panic!("an unanswered session wraps");
    };
    assert_eq!(second.base_key, init.base_key);
    let bob = accept(
        &SessionRecord::new(a.identity.public_key_bytes()),
        &b,
        &init,
    );
    let rng = TestRandom::for_message(&second.message.ciphertext);
    let (mut bob, plaintext) = bob.receive_known_init(&second, AD, NOW, &rng).unwrap();
    assert_eq!(plaintext, b"second");
    assert!(matches!(
        bob.receive_known_init(&second, AD, NOW, &rng),
        Err(CryptoError::DuplicateMessage)
    ));

    assert!(matches!(
        alice.encrypt(b"x", AD, NOW + PREKEY_WRAP_MAX_AGE_MS + 1),
        Err(CryptoError::NoSession(_))
    ));
    assert!(matches!(
        send_at(&mut alice.clone(), b"x", NOW + PREKEY_WRAP_MAX_AGE_MS),
        Outgoing::PreKey(_)
    ));

    let pong = send(&mut bob, b"pong");
    receive(&mut alice, &pong).unwrap();
    assert!(matches!(
        send_at(&mut alice, b"answered", NOW + PREKEY_WRAP_MAX_AGE_MS * 2),
        Outgoing::Message(_)
    ));
}

/// An init whose first message does not authenticate leaves the record as
/// it was (the candidate is built apart from it).
#[test]
fn a_failed_initiation_changes_nothing() {
    let (a, b, alice, _) = established();
    let (_, mut init) = initiate(
        &b,
        &a,
        &SessionRecord::new(a.identity.public_key_bytes()),
        50,
    );
    init.message.ciphertext[0] ^= 1;
    let before = alice.to_bytes();
    let keys = a.keys_for(init.one_time_prekey_id, init.pq_prekey);
    assert!(alice
        .insert_from_init(&init, &keys, AD, NOW, &TestRandom::new(1))
        .is_err());
    assert_eq!(*before, *alice.to_bytes());
}

/// The stored form keeps the pending init and the breaker's state.
#[test]
fn the_stored_form_keeps_the_pending_init_and_the_counter() {
    let (a, b, alice, mut bob) = established();
    let (mut alice, _) = initiate(&a, &b, &alice, 60);
    let old = send(&mut bob, b"old");
    receive(&mut alice, &old).unwrap();
    let stored = SessionRecord::from_bytes(&alice.to_bytes()).unwrap();
    assert_eq!(*stored.to_bytes(), *alice.to_bytes());
    assert_eq!(stored.direction_changes, 1);
    assert!(stored.active.as_ref().unwrap().pending.is_some());
}
