use super::*;
use rekindle_utils::OsRandom;

#[test]
fn round_trip_and_refusals() {
    let key = mint_first(&OsRandom);
    let content = seal(&key, [1; 16], "ann", b"hello").expect("seal");
    assert_eq!(content.epoch, FIRST_EPOCH);
    assert_eq!(open(&key, "ann", &content).expect("open"), b"hello");
    assert!(matches!(
        open(&key, "bea", &content),
        Err(DmError::Crypto(_))
    ));
    let mut other_group = content.clone();
    other_group.group_id = [2; 16];
    assert!(matches!(
        open(&key, "ann", &other_group),
        Err(DmError::Crypto(_))
    ));
    let next = rotate(&key, &OsRandom).expect("rotate");
    assert_eq!(next.epoch(), FIRST_EPOCH + 1);
    assert!(matches!(
        open(&next, "ann", &content),
        Err(DmError::StaleEpoch { got: 0, current: 1 })
    ));
    let newer = seal(&next, [1; 16], "ann", b"later").expect("seal");
    assert!(matches!(
        open(&key, "ann", &newer),
        Err(DmError::KeyMissing { epoch: 1, .. })
    ));
}

#[test]
fn exhausted() {
    let last = GroupSenderKey::new(u32::MAX, [0; SENDER_KEY_LEN]);
    assert!(matches!(
        rotate(&last, &OsRandom),
        Err(DmError::EpochExhausted)
    ));
}
