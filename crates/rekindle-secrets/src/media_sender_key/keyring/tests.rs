use std::time::{Duration, Instant};

use super::*;

fn secret(b: u8) -> Zeroizing<[u8; 32]> {
    Zeroizing::new([b; 32])
}

#[test]
fn a_join_within_the_grace_gets_the_current_key() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new(now);
    let current = keys.send_key(now);
    match keys.on_join(now + Duration::from_secs(3)) {
        JoinShare::Current(k) => assert_eq!(k.index, current.index),
        JoinShare::Rotated(_) => panic!("a young key is shared, not rotated"),
    }
}

#[test]
fn a_join_past_the_grace_rotates_and_the_new_key_waits_its_delay() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new(now);
    let first = keys.send_key(now);
    let later = now + SHARE_GRACE + Duration::from_secs(1);
    let JoinShare::Rotated(rotated) = keys.on_join(later) else {
        panic!("a stale key is rotated");
    };
    assert_eq!(rotated.index, first.index + 1);
    assert_eq!(
        keys.send_key(later).index,
        first.index,
        "not used before the delay"
    );
    assert_eq!(keys.send_key(later + USE_DELAY).index, rotated.index);
}

#[test]
fn leaves_during_the_delay_coalesce_into_one_rotation() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new(now);
    let first = keys.send_key(now).index;
    let a = keys.on_leave(now);
    let b = keys.on_leave(now + Duration::from_secs(1));
    assert_eq!(a.index, first + 1);
    assert_eq!(
        b.index, a.index,
        "the pending rotation is replaced, not stacked"
    );
    assert_ne!(*a.secret, *b.secret);
    let used = keys.send_key(now + Duration::from_secs(1) + USE_DELAY);
    assert_eq!(*used.secret, *b.secret);
}

#[test]
fn a_join_during_a_pending_rotation_gets_the_pending_key() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new(now);
    let rotated = keys.on_leave(now);
    match keys.on_join(now) {
        JoinShare::Current(k) => assert_eq!(*k.secret, *rotated.secret),
        JoinShare::Rotated(_) => panic!("one rotation at a time"),
    }
}

#[test]
fn received_keys_match_by_index_low_bits_and_keep_the_last_five() {
    let keys = ChannelSenderKeys::default();
    for i in 0..7u8 {
        keys.install("alice", 0x1000 + u64::from(i), secret(i));
    }
    assert_eq!(*keys.receive_secret("alice", 0x06).unwrap(), [6; 32]);
    assert!(
        keys.receive_secret("alice", 0x01).is_err(),
        "the oldest are forgotten"
    );
    // A rejoin's key under an index already held replaces it.
    keys.install("alice", 0x1006, secret(9));
    assert_eq!(*keys.receive_secret("alice", 0x06).unwrap(), [9; 32]);
}

#[test]
fn a_missing_key_names_the_next_index_with_those_low_bits() {
    let keys = ChannelSenderKeys::default();
    keys.install("bob", 0x2_05, secret(1));
    assert_eq!(
        keys.receive_secret("bob", 0x07),
        Err(MissingKey {
            sender: "bob".into(),
            index: 0x2_07
        })
    );
    assert_eq!(keys.receive_secret("bob", 0x03).unwrap_err().index, 0x3_03);
    assert_eq!(keys.receive_secret("carol", 0x09).unwrap_err().index, 0x09);
    keys.forget("bob");
    assert!(keys.receive_secret("bob", 0x05).is_err());
}

#[test]
fn an_exact_index_is_found_or_named() {
    let keys = ChannelSenderKeys::default();
    keys.install("alice", 0x3_10, secret(4));
    assert_eq!(*keys.secret_at("alice", 0x3_10).unwrap(), [4; 32]);
    assert_eq!(
        keys.secret_at("alice", 0x4_10),
        Err(MissingKey {
            sender: "alice".into(),
            index: 0x4_10
        })
    );
}

#[test]
fn sessions_start_at_independent_indices() {
    let a = ChannelSenderKeys::default().send_key(Instant::now()).index;
    let b = ChannelSenderKeys::default().send_key(Instant::now()).index;
    // Random 32-bit starts: equal only by a 1-in-2^32 chance.
    assert_ne!(a, b);
}
