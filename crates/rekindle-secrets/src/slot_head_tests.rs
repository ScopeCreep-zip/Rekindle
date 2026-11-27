use super::slot_head_digest;

/// Every input moves the digest: a head names one record, one slot, one
/// seq and one page, and the record key is length-prefixed so a key's
/// tail cannot pass as the subkey.
#[test]
fn every_field_is_committed() {
    let base = slot_head_digest("VLD0:a", 2, 7, b"page");
    assert_eq!(base, slot_head_digest("VLD0:a", 2, 7, b"page"));
    for other in [
        slot_head_digest("VLD0:b", 2, 7, b"page"),
        slot_head_digest("VLD0:a", 3, 7, b"page"),
        slot_head_digest("VLD0:a", 2, 8, b"page"),
        slot_head_digest("VLD0:a", 2, 7, b"pagf"),
        slot_head_digest("VLD0:a\u{2}", 0, 7, b"page"),
    ] {
        assert_ne!(base, other);
    }
}
