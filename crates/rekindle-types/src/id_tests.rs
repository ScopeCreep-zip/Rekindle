use super::*;

#[test]
fn pseudonym_key_serde_roundtrip() {
    let key = PseudonymKey([42u8; 32]);
    let json = serde_json::to_string(&key).unwrap();
    let back: PseudonymKey = serde_json::from_str(&json).unwrap();
    assert_eq!(key, back);
}

#[test]
fn distinct_types_not_mixable() {
    // This is a compile-time guarantee — these are different types.
    // If someone tries to pass ChannelId where RoleId is expected, it won't compile.
    let ch = ChannelId([1u8; 16]);
    let role = RoleId([1u8; 16]);
    // Identical bytes, different types: the newtypes are the whole
    // point, so assert on the bytes rather than binding and
    // discarding — a discarded binding proves nothing at runtime.
    assert_eq!(ch.0, role.0);
}

#[test]
fn slot_index_range() {
    // Valid range is 0..255 for a 255-member SMPL record
    let slot = SlotIndex(254);
    assert_eq!(slot.0, 254);
}
