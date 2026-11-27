use super::{BASE32_RAMP, FRIEND_TOKEN, HEX_KEY_64, HEX_RAMP};

#[test]
fn every_secret_shaped_fixture_carries_its_marker() {
    assert_eq!(HEX_KEY_64, HEX_RAMP.repeat(4));
    assert!(HEX_KEY_64.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(FRIEND_TOKEN.len(), 26);
    assert!(FRIEND_TOKEN.starts_with(BASE32_RAMP));
    assert!(FRIEND_TOKEN
        .bytes()
        .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)));
}
