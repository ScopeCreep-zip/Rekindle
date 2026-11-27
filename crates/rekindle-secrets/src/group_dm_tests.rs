use super::*;
use rekindle_utils::OsRandom;

const GROUP: [u8; GROUP_ID_LEN] = [7; GROUP_ID_LEN];

#[test]
fn round_trip() {
    let key = GroupSenderKey::mint(0, &OsRandom);
    let sealed = seal(&key, &GROUP, "ann", b"hi").expect("seal");
    assert_eq!(open(&key, &GROUP, "ann", &sealed).expect("open"), b"hi");
}

#[test]
fn bound_to_group_sender_and_epoch() {
    let key = GroupSenderKey::mint(3, &OsRandom);
    let sealed = seal(&key, &GROUP, "ann", b"hi").expect("seal");
    assert!(open(&key, &[8; GROUP_ID_LEN], "ann", &sealed).is_err());
    assert!(open(&key, &GROUP, "bea", &sealed).is_err());
    let same_bytes_other_epoch = GroupSenderKey::new(4, *key.as_bytes());
    assert!(open(&same_bytes_other_epoch, &GROUP, "ann", &sealed).is_err());
    let next = key.next(&OsRandom).expect("next");
    assert_eq!(next.epoch(), 4);
    assert!(open(&next, &GROUP, "ann", &sealed).is_err());
    assert!(open(&key, &GROUP, "ann", &sealed[..20]).is_err());
}

#[test]
fn epochs_end() {
    assert!(GroupSenderKey::mint(u32::MAX, &OsRandom)
        .next(&OsRandom)
        .is_none());
}

#[test]
fn debug_redacts() {
    let key = GroupSenderKey::new(1, [0xAB; SENDER_KEY_LEN]);
    assert!(!format!("{key:?}").contains("171"));
}
