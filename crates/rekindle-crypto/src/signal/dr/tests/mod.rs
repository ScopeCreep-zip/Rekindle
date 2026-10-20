//! The ratchet's own cases and libsignal's bounds; `differential` runs it
//! against the §3.5 reference model in `model`.

mod differential;
mod model;

use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use zeroize::Zeroizing;

use super::{Header, SessionState, MAX_FORWARD_JUMPS, MAX_MESSAGE_KEYS, MAX_RECEIVER_CHAINS};
use crate::error::CryptoError;
use crate::signal::pqxdh::agreement::{SessionKeys, AD_LEN};
use crate::signal::test_support::TestRandom;

const SK: [u8; 32] = [7; 32];
const AD: [u8; AD_LEN] = [3; AD_LEN];
const BOB_SPK: [u8; 32] = [2; 32];

fn keys() -> SessionKeys {
    SessionKeys {
        sk_ec: Zeroizing::new(SK),
        sk_scka: Zeroizing::new([8; 32]),
    }
}

fn bob_spk_public() -> [u8; 32] {
    X25519Public::from(&StaticSecret::from(BOB_SPK)).to_bytes()
}

/// An initiator and a responder over the same `SKec`, as PQXDH leaves them.
fn pair(seed: u64) -> (SessionState, SessionState) {
    let alice = SessionState::initiator(
        &keys(),
        AD,
        [1; 32],
        bob_spk_public(),
        &TestRandom::new(seed),
    )
    .unwrap();
    let bob = SessionState::responder(&keys(), AD, [1; 32], &StaticSecret::from(BOB_SPK));
    (alice, bob)
}

/// Decrypt on a clone and keep it only on success, as the record does.
fn receive(
    state: &mut SessionState,
    message: &super::RatchetMessage,
) -> Result<Vec<u8>, CryptoError> {
    let mut trial = state.clone();
    let plaintext = trial.decrypt(message, b"", &TestRandom::for_message(&message.ciphertext))?;
    *state = trial;
    Ok(plaintext)
}

#[test]
fn messages_flow_both_ways_after_the_first() {
    let (mut alice, mut bob) = pair(1);
    let first = alice.encrypt(b"hi bob", b"").unwrap();
    assert_eq!(receive(&mut bob, &first).unwrap(), b"hi bob");
    for i in 0..6u8 {
        let (from, to) = if i % 2 == 0 {
            (&mut bob, &mut alice)
        } else {
            (&mut alice, &mut bob)
        };
        let message = from.encrypt(&[i; 9], b"").unwrap();
        assert_eq!(receive(to, &message).unwrap(), [i; 9]);
    }
}

#[test]
fn the_responder_cannot_send_before_it_receives() {
    let (_, mut bob) = pair(2);
    assert!(bob.encrypt(b"too early", b"").is_err());
}

#[test]
fn a_late_message_from_the_previous_chain_decrypts() {
    let (mut alice, mut bob) = pair(3);
    let late = alice.encrypt(b"late", b"").unwrap();
    let on_time = alice.encrypt(b"on time", b"").unwrap();
    receive(&mut bob, &on_time).unwrap();
    let reply = bob.encrypt(b"reply", b"").unwrap();
    receive(&mut alice, &reply).unwrap();
    let next = alice.encrypt(b"next chain", b"").unwrap();
    assert_eq!(next.header.pn, 2);
    receive(&mut bob, &next).unwrap();
    assert_eq!(receive(&mut bob, &late).unwrap(), b"late");
}

#[test]
fn a_replayed_message_is_a_duplicate_and_changes_nothing() {
    let (mut alice, mut bob) = pair(4);
    let skipped = alice.encrypt(b"zero", b"").unwrap();
    let message = alice.encrypt(b"one", b"").unwrap();
    receive(&mut bob, &message).unwrap();
    assert!(matches!(
        receive(&mut bob, &message),
        Err(CryptoError::DuplicateMessage)
    ));
    receive(&mut bob, &skipped).unwrap();
    assert!(matches!(
        receive(&mut bob, &skipped),
        Err(CryptoError::DuplicateMessage)
    ));
}

#[test]
fn a_tampered_message_or_other_ad_fails_and_changes_nothing() {
    let (mut alice, mut bob) = pair(5);
    let message = alice.encrypt(b"payload", b"type").unwrap();
    let mut tampered = message.clone();
    tampered.ciphertext[0] ^= 1;
    assert!(receive(&mut bob, &tampered).is_err());
    let mut trial = bob.clone();
    assert!(trial
        .decrypt(&message, b"other", &TestRandom::new(1))
        .is_err());
    let mut header = message.clone();
    header.header.pn += 1;
    assert!(receive(&mut bob, &header).is_err());
    let mut trial = bob.clone();
    assert_eq!(
        trial
            .decrypt(&message, b"type", &TestRandom::new(1))
            .unwrap(),
        b"payload"
    );
}

#[test]
fn a_forward_jump_past_the_bound_is_refused_before_any_key_is_derived() {
    let (mut alice, mut bob) = pair(6);
    let mut message = alice.encrypt(b"x", b"").unwrap();
    message.header.n = MAX_FORWARD_JUMPS + 1;
    let err = receive(&mut bob, &message).unwrap_err();
    assert!(err.to_string().contains("ahead"), "{err}");
    message.header.n = MAX_FORWARD_JUMPS;
    let err = receive(&mut bob, &message).unwrap_err();
    assert!(err.to_string().contains("authenticate"), "{err}");
}

/// The stored form, to show a refusal changed nothing.
fn stored(state: &SessionState) -> Vec<u8> {
    let mut bytes = Vec::new();
    state.write(&mut bytes);
    bytes
}

/// Low-order X25519 points: `0`, `1` and the order-8 point (RFC 7748 §6.1
/// requires refusing the all-zero shared secret they give).
fn low_order_points() -> [[u8; 32]; 3] {
    let mut one = [0u8; 32];
    one[0] = 1;
    let order8: [u8; 32] =
        hex::decode("e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800")
            .unwrap()
            .try_into()
            .unwrap();
    [[0; 32], one, order8]
}

/// A message under a new ratchet key with `n` (or `pn`) past the bound is
/// refused before the DH ratchet runs: the state, decrypted in place, is
/// byte-identical, and a low-order key is refused for the jump, not the
/// agreement (which never ran).
#[test]
fn a_new_chain_past_the_bound_is_refused_before_the_ratchet_runs() {
    let (mut alice, mut bob) = pair(10);
    receive(&mut bob, &alice.encrypt(b"first", b"").unwrap()).unwrap();
    let before = stored(&bob);
    let fresh = X25519Public::from(&StaticSecret::from([9; 32])).to_bytes();
    for dh in [fresh, low_order_points()[0]] {
        for (pn, n) in [(0, MAX_FORWARD_JUMPS + 1), (MAX_FORWARD_JUMPS + 2, 0)] {
            let mut message = alice.encrypt(b"x", b"").unwrap();
            message.header = Header { dh, pn, n };
            let err = bob.decrypt(&message, b"", &TestRandom::new(1)).unwrap_err();
            assert!(err.to_string().contains("ahead"), "{err}");
            assert_eq!(stored(&bob), before);
        }
    }
}

#[test]
fn a_low_order_ratchet_key_is_refused_and_changes_nothing() {
    let (mut alice, mut bob) = pair(11);
    receive(&mut bob, &alice.encrypt(b"first", b"").unwrap()).unwrap();
    let before = stored(&bob);
    for dh in low_order_points() {
        let mut message = alice.encrypt(b"x", b"").unwrap();
        message.header.dh = dh;
        let err = receive(&mut bob, &message).unwrap_err();
        assert!(err.to_string().contains("non-contributory"), "{err}");
        assert_eq!(stored(&bob), before);
    }
    let next = alice.encrypt(b"still", b"").unwrap();
    assert_eq!(receive(&mut bob, &next).unwrap(), b"still");
}

#[test]
fn a_chain_keeps_only_the_newest_skipped_keys() {
    let (mut alice, mut bob) = pair(7);
    let messages: Vec<_> = (0..MAX_MESSAGE_KEYS + 3)
        .map(|i| alice.encrypt(&i.to_be_bytes(), b"").unwrap())
        .collect();
    receive(&mut bob, messages.last().unwrap()).unwrap();
    for dropped in &messages[..2] {
        assert!(matches!(
            receive(&mut bob, dropped),
            Err(CryptoError::DuplicateMessage)
        ));
    }
    for kept in [&messages[2], &messages[MAX_MESSAGE_KEYS + 1]] {
        receive(&mut bob, kept).unwrap();
    }
}

/// `x0` is skipped on the responder's first receiving chain; `rounds`
/// round trips later it is delivered.
fn deliver_after_rounds(rounds: usize) -> Result<Vec<u8>, CryptoError> {
    let (mut alice, mut bob) = pair(8);
    let x0 = alice.encrypt(b"x0", b"").unwrap();
    let x1 = alice.encrypt(b"x1", b"").unwrap();
    receive(&mut bob, &x1).unwrap();
    for _ in 0..rounds {
        let reply = bob.encrypt(b"r", b"").unwrap();
        receive(&mut alice, &reply).unwrap();
        let next = alice.encrypt(b"a", b"").unwrap();
        receive(&mut bob, &next).unwrap();
    }
    receive(&mut bob, &x0)
}

#[test]
fn only_the_newest_receiving_chains_are_kept() {
    assert_eq!(
        deliver_after_rounds(MAX_RECEIVER_CHAINS - 1).unwrap(),
        b"x0"
    );
    assert!(deliver_after_rounds(MAX_RECEIVER_CHAINS).is_err());
}

#[test]
fn the_stored_form_round_trips_mid_conversation() {
    let (mut alice, mut bob) = pair(9);
    let skipped = alice.encrypt(b"skipped", b"").unwrap();
    let message = alice.encrypt(b"m", b"").unwrap();
    receive(&mut bob, &message).unwrap();

    let mut bytes = Vec::new();
    bob.write(&mut bytes);
    let mut reader = super::Reader::new(&bytes);
    let mut restored = SessionState::read(&mut reader).unwrap();
    assert!(reader.is_empty());
    assert_eq!(receive(&mut restored, &skipped).unwrap(), b"skipped");
    let reply = restored.encrypt(b"reply", b"").unwrap();
    assert_eq!(receive(&mut alice, &reply).unwrap(), b"reply");

    for cut in [0, 1, 66, bytes.len() - 1] {
        assert!(SessionState::read(&mut super::Reader::new(&bytes[..cut])).is_err());
    }
}
