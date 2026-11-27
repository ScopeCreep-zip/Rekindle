use super::*;

fn pad_entries(real_blobs: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let target_len = real_blobs
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(DUMMY_ENTRY_MIN_SIZE);
    let mut entries = real_blobs;
    while entries.len() < RELAY_POOL_PAD_SIZE {
        let mut dummy = vec![0u8; target_len];
        dummy[0] = 0xFF;
        entries.push(dummy);
    }
    entries
}

#[test]
fn padding_marks_dummy_entries() {
    let real_blobs: Vec<Vec<u8>> = vec![vec![1, 2, 3], vec![4, 5, 6]];
    let entries = pad_entries(real_blobs.clone());
    assert_eq!(entries.len(), RELAY_POOL_PAD_SIZE);
    assert_eq!(entries[0], real_blobs[0]);
    for entry in &entries[real_blobs.len()..] {
        assert_eq!(entry[0], 0xFF);
    }
}

#[test]
fn dummies_meet_min_size_when_pool_is_empty() {
    let entries = pad_entries(Vec::new());
    assert_eq!(entries.len(), RELAY_POOL_PAD_SIZE);
    for entry in &entries {
        assert_eq!(entry.len(), DUMMY_ENTRY_MIN_SIZE);
        assert_eq!(entry[0], 0xFF);
    }
}

#[test]
fn dummies_track_largest_real_entry_when_above_min() {
    // A real entry larger than DUMMY_ENTRY_MIN_SIZE forces dummies
    // up so they remain indistinguishable by length.
    let big = vec![7u8; DUMMY_ENTRY_MIN_SIZE * 2];
    let entries = pad_entries(vec![big.clone()]);
    for entry in entries.iter().skip(1) {
        assert_eq!(entry.len(), big.len());
    }
}
