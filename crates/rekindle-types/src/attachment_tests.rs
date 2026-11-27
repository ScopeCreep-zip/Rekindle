use super::*;

#[test]
fn bitmap_set_has_count() {
    let mut bm = AttachmentBitmap::new(20);
    assert!(bm.set(0));
    assert!(bm.set(7));
    assert!(bm.set(8));
    assert!(bm.set(19));
    assert!(!bm.set(20)); // out of range no-op
    assert!(bm.has(0) && bm.has(7) && bm.has(8) && bm.has(19));
    assert!(!bm.has(1));
    assert_eq!(bm.count(), 4);
}

#[test]
fn bitmap_full_complete() {
    let bm = AttachmentBitmap::full(33);
    assert!(bm.is_complete());
    assert_eq!(bm.count(), 33);
}

#[test]
fn bitmap_intersect() {
    let mut a = AttachmentBitmap::new(10);
    let mut b = AttachmentBitmap::new(10);
    for i in [0, 2, 4, 6, 8] {
        a.set(i);
    }
    for i in [0, 1, 4, 9] {
        b.set(i);
    }
    assert_eq!(a.intersect(&b), vec![0, 4]);
}

#[test]
fn bitmap_serde_roundtrip() {
    let mut bm = AttachmentBitmap::new(17);
    bm.set(3);
    bm.set(15);
    let json = serde_json::to_string(&bm).unwrap();
    let back: AttachmentBitmap = serde_json::from_str(&json).unwrap();
    assert_eq!(bm, back);
}

#[test]
fn bitmap_from_bytes_validates_length() {
    // chunk_count 17 → expected 3 bytes
    assert!(AttachmentBitmap::from_bytes(vec![0; 4], 17).is_none());
    assert!(AttachmentBitmap::from_bytes(vec![0; 3], 17).is_some());
}
