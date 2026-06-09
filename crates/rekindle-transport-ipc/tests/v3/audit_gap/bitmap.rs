use rekindle_transport_ipc::v3::audit::gap::MissingBitmapBuilder;

#[test]
fn bitmap_encodes_specific_missing_frames() {
    let mut builder = MissingBitmapBuilder::new(50, 60);
    builder.mark_missing(52);
    builder.mark_missing(55);
    let bitmap = builder.build();
    assert!(bitmap.is_missing(52));
    assert!(bitmap.is_missing(55));
    assert!(!bitmap.is_missing(50));
    assert!(!bitmap.is_missing(51));
    assert!(!bitmap.is_missing(53));
}

#[test]
fn bitmap_all_missing() {
    let mut builder = MissingBitmapBuilder::new(10, 17); // 8 frames
    for seq in 10..=17 {
        builder.mark_missing(seq);
    }
    let bitmap = builder.build();
    for seq in 10..=17 {
        assert!(bitmap.is_missing(seq), "seq {seq} must be missing");
    }
}

#[test]
fn bitmap_none_missing() {
    let builder = MissingBitmapBuilder::new(10, 17);
    let bitmap = builder.build();
    for seq in 10..=17 {
        assert!(!bitmap.is_missing(seq), "seq {seq} must not be missing");
    }
    assert_eq!(bitmap.missing_count(), 0);
}

#[test]
fn bitmap_single_frame_missing() {
    let mut builder = MissingBitmapBuilder::new(0, 7);
    builder.mark_missing(3);
    let bitmap = builder.build();
    assert_eq!(bitmap.missing_count(), 1);
    assert!(bitmap.is_missing(3));
    assert!(!bitmap.is_missing(0));
    assert!(!bitmap.is_missing(7));
}

#[test]
fn bitmap_length_matches_range() {
    let builder = MissingBitmapBuilder::new(50, 60); // 11 frames, needs ceil(11/8) = 2 bytes
    let bitmap = builder.build();
    assert_eq!(bitmap.as_bytes().len(), 2);
}

#[test]
fn bitmap_length_exact_byte_boundary() {
    let builder = MissingBitmapBuilder::new(0, 7); // 8 frames = exactly 1 byte
    let bitmap = builder.build();
    assert_eq!(bitmap.as_bytes().len(), 1);
}

#[test]
fn bitmap_length_16_frames() {
    let builder = MissingBitmapBuilder::new(0, 15); // 16 frames = 2 bytes
    let bitmap = builder.build();
    assert_eq!(bitmap.as_bytes().len(), 2);
}

#[test]
fn missing_count_matches_popcount() {
    let mut builder = MissingBitmapBuilder::new(0, 31); // 32 frames
    builder.mark_missing(0);
    builder.mark_missing(7);
    builder.mark_missing(15);
    builder.mark_missing(31);
    let bitmap = builder.build();
    assert_eq!(bitmap.missing_count(), 4);
}

#[test]
fn iterate_missing_seqs() {
    let mut builder = MissingBitmapBuilder::new(100, 107);
    builder.mark_missing(102);
    builder.mark_missing(105);
    let bitmap = builder.build();
    let missing: Vec<u64> = bitmap.missing_seqs().collect();
    assert_eq!(missing, vec![102, 105]);
}

#[test]
fn out_of_range_mark_ignored() {
    let mut builder = MissingBitmapBuilder::new(10, 20);
    builder.mark_missing(5);  // below range
    builder.mark_missing(25); // above range
    let bitmap = builder.build();
    assert_eq!(bitmap.missing_count(), 0);
}
