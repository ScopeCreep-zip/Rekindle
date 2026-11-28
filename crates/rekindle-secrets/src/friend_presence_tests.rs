use super::{open, seal, PresenceKey};

const OWNER: &str = "aa11";

#[test]
fn a_sealed_value_opens_with_its_key_only() {
    let key = PresenceKey::new(3, [7; 32]);
    let sealed = seal(&key, OWNER, 2, b"\x01status").unwrap();
    assert_eq!(open(&key, OWNER, 2, &sealed).unwrap(), b"\x01status");
    // Another key of the same epoch, the next epoch, another subkey,
    // another owner, a flipped bit: none opens.
    assert!(open(&PresenceKey::new(3, [8; 32]), OWNER, 2, &sealed).is_err());
    assert!(open(&PresenceKey::new(4, [7; 32]), OWNER, 2, &sealed).is_err());
    assert!(open(&key, OWNER, 4, &sealed).is_err());
    assert!(open(&key, "bb22", 2, &sealed).is_err());
    let mut flipped = sealed.clone();
    *flipped.last_mut().unwrap() ^= 1;
    assert!(open(&key, OWNER, 2, &flipped).is_err());
    // A plaintext value is never shown.
    assert!(open(&key, OWNER, 2, &[0]).is_err());
    assert!(open(&key, OWNER, 2, &[]).is_err());
}

#[test]
fn the_next_epoch_is_a_new_key() {
    let rng = rekindle_utils::OsRandom;
    let first = PresenceKey::mint(1, &rng);
    let next = first.next(&rng).unwrap();
    assert_eq!(next.epoch(), 2);
    assert_ne!(next.as_bytes(), first.as_bytes());
    assert!(PresenceKey::new(u32::MAX, [0; 32]).next(&rng).is_none());
    assert_eq!(format!("{first:?}"), "PresenceKey(<redacted>)");
}
