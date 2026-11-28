use super::Cursor;

#[test]
fn base128_reads_the_spec_forms() {
    let ok = |b: &[u8]| Cursor { data: b, pos: 0 }.base128().ok();
    assert_eq!(ok(&[0x3F]), Some(63));
    assert_eq!(ok(&[0x81, 0x00]), Some(128));
    assert_eq!(ok(&[0x8F, 0xFF, 0xFF, 0xFF, 0x7F]), Some(u32::MAX));
    assert_eq!(ok(&[0x80, 0x01]), None, "leading zero");
    assert_eq!(ok(&[0x90, 0x80, 0x80, 0x80, 0x00]), None, "overflow");
    assert_eq!(ok(&[0x81, 0x81, 0x81, 0x81, 0x81, 0x01]), None, "six bytes");
}
