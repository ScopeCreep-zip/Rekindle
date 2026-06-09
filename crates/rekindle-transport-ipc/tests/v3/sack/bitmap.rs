use rekindle_transport_ipc::v3::stream::sack::{SackBitmap, SackBuilder};

#[test]
fn empty_bitmap_means_no_gaps() {
    let bm = SackBitmap::new(100, &[]);
    assert_eq!(bm.acknowledged_chunks().count(), 0);
}

#[test]
fn single_bit_set() {
    // Bit 0 → chunk cumulative_through + 1 = 101
    let bm = SackBitmap::new(100, &[0b0000_0001]);
    let acked: Vec<u32> = bm.acknowledged_chunks().collect();
    assert_eq!(acked, vec![101]);
}

#[test]
fn multiple_bits_set() {
    // Bits 0, 2, 4 → chunks 101, 103, 105
    let bm = SackBitmap::new(100, &[0b0001_0101]);
    let acked: Vec<u32> = bm.acknowledged_chunks().collect();
    assert_eq!(acked, vec![101, 103, 105]);
}

#[test]
fn all_bits_set_in_byte() {
    let bm = SackBitmap::new(100, &[0xFF]);
    let acked: Vec<u32> = bm.acknowledged_chunks().collect();
    assert_eq!(acked, vec![101, 102, 103, 104, 105, 106, 107, 108]);
}

#[test]
fn bitmap_spans_multiple_bytes() {
    let mut bitmap = vec![0u8; 128]; // 1024 chunk positions
    bitmap[0] = 0xFF;   // chunks 101-108
    bitmap[127] = 0x01; // chunk 1117 (100 + 127*8 + 1)
    let bm = SackBitmap::new(100, &bitmap);
    let acked: Vec<u32> = bm.acknowledged_chunks().collect();
    assert!(acked.contains(&101));
    assert!(acked.contains(&108));
    assert!(acked.contains(&1117));
    assert_eq!(acked.len(), 9); // 8 from first byte + 1 from last
}

#[test]
fn cumulative_plus_sack_covers_non_contiguous() {
    // Cumulative through 100 means 0-100 are all acked.
    // SACK bits for 102 and 105 (relative positions 1 and 4)
    let bm = SackBitmap::new(100, &[0b0001_0010]); // bits 1 and 4
    let acked: Vec<u32> = bm.acknowledged_chunks().collect();
    assert_eq!(acked, vec![102, 105]);
}

#[test]
fn builder_tracks_received_chunks() {
    let mut builder = SackBuilder::new(100);
    builder.mark_received(102);
    builder.mark_received(105);
    builder.mark_received(110);

    let (cumulative_through, bitmap) = builder.build();
    assert_eq!(cumulative_through, 100);

    let bm = SackBitmap::new(cumulative_through, &bitmap);
    let acked: Vec<u32> = bm.acknowledged_chunks().collect();
    assert!(acked.contains(&102));
    assert!(acked.contains(&105));
    assert!(acked.contains(&110));
    assert!(!acked.contains(&101)); // not marked
    assert!(!acked.contains(&103)); // not marked
}

#[test]
fn builder_ignores_chunks_below_cumulative() {
    let mut builder = SackBuilder::new(100);
    builder.mark_received(50); // below cumulative — should be ignored
    builder.mark_received(100); // at cumulative — should be ignored
    builder.mark_received(101); // above — should be tracked

    let (_, bitmap) = builder.build();
    let bm = SackBitmap::new(100, &bitmap);
    let acked: Vec<u32> = bm.acknowledged_chunks().collect();
    assert_eq!(acked, vec![101]);
}

#[test]
fn bitmap_bit_positions_are_zero_indexed() {
    // chunk cumulative_through + 1 = bit 0
    // chunk cumulative_through + 2 = bit 1
    let bm = SackBitmap::new(0, &[0b0000_0011]); // bits 0 and 1
    let acked: Vec<u32> = bm.acknowledged_chunks().collect();
    assert_eq!(acked, vec![1, 2]);
}
