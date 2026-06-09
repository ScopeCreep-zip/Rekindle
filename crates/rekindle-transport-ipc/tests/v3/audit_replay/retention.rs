use rekindle_transport_ipc::v3::audit::retention::{RetentionBuffer, RetentionConfig};

fn default_config() -> RetentionConfig {
    RetentionConfig {
        max_frames: 100,
        max_bytes: 1_048_576, // 1 MiB
    }
}

fn make_frame(seq: u64, size: usize) -> Vec<u8> {
    let mut frame = vec![seq as u8; size];
    frame[0..8].copy_from_slice(&seq.to_le_bytes());
    frame
}

#[test]
fn retained_frame_retrievable() {
    let mut buf = RetentionBuffer::new(default_config());
    let frame = make_frame(0, 100);
    buf.store(0, frame.clone());
    let retrieved = buf.get(0);
    assert_eq!(retrieved, Some(frame.as_slice()));
}

#[test]
fn retention_evicts_oldest_by_count() {
    let config = RetentionConfig { max_frames: 3, max_bytes: 1_048_576 };
    let mut buf = RetentionBuffer::new(config);
    buf.store(0, make_frame(0, 10));
    buf.store(1, make_frame(1, 10));
    buf.store(2, make_frame(2, 10));
    buf.store(3, make_frame(3, 10)); // evicts seq 0
    assert!(buf.get(0).is_none(), "seq 0 must be evicted");
    assert!(buf.get(1).is_some());
    assert!(buf.get(2).is_some());
    assert!(buf.get(3).is_some());
}

#[test]
fn retention_evicts_by_bytes() {
    let config = RetentionConfig { max_frames: 1000, max_bytes: 250 };
    let mut buf = RetentionBuffer::new(config);
    buf.store(0, make_frame(0, 100));
    buf.store(1, make_frame(1, 100));
    buf.store(2, make_frame(2, 100)); // total 300 > 250, evicts seq 0
    assert!(buf.get(0).is_none(), "seq 0 must be evicted due to byte limit");
    assert!(buf.get(1).is_some());
    assert!(buf.get(2).is_some());
}

#[test]
fn evicted_frame_not_retrievable() {
    let config = RetentionConfig { max_frames: 2, max_bytes: 1_048_576 };
    let mut buf = RetentionBuffer::new(config);
    buf.store(0, make_frame(0, 10));
    buf.store(1, make_frame(1, 10));
    buf.store(2, make_frame(2, 10)); // evicts seq 0
    assert!(buf.get(0).is_none());
}

#[test]
fn retention_preserves_full_bytes() {
    let mut buf = RetentionBuffer::new(default_config());
    let frame = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE];
    buf.store(42, frame.clone());
    assert_eq!(buf.get(42), Some(frame.as_slice()));
}

#[test]
fn retention_capacity_configurable() {
    let config = RetentionConfig { max_frames: 5, max_bytes: 1_048_576 };
    let mut buf = RetentionBuffer::new(config);
    for seq in 0..5 {
        buf.store(seq, make_frame(seq, 10));
    }
    // All 5 present
    for seq in 0..5 {
        assert!(buf.get(seq).is_some(), "seq {seq} must be present");
    }
    // 6th evicts oldest
    buf.store(5, make_frame(5, 10));
    assert!(buf.get(0).is_none());
    assert!(buf.get(5).is_some());
}

#[test]
fn get_range_returns_only_present() {
    let config = RetentionConfig { max_frames: 10, max_bytes: 1_048_576 };
    let mut buf = RetentionBuffer::new(config);
    buf.store(0, make_frame(0, 10));
    buf.store(1, make_frame(1, 10));
    // seq 2 not stored
    buf.store(3, make_frame(3, 10));

    let frames = buf.get_range(0, 3);
    assert_eq!(frames.len(), 4); // 0, 1, 2, 3 — inclusive range
    assert!(frames[0].is_some()); // seq 0
    assert!(frames[1].is_some()); // seq 1
    assert!(frames[2].is_none()); // seq 2 not stored
    assert!(frames[3].is_some()); // seq 3
}

#[test]
fn stored_bytes_tracked() {
    let mut buf = RetentionBuffer::new(default_config());
    assert_eq!(buf.total_bytes(), 0);
    buf.store(0, make_frame(0, 100));
    assert_eq!(buf.total_bytes(), 100);
    buf.store(1, make_frame(1, 200));
    assert_eq!(buf.total_bytes(), 300);
}

#[test]
fn frame_count_tracked() {
    let mut buf = RetentionBuffer::new(default_config());
    assert_eq!(buf.frame_count(), 0);
    buf.store(0, make_frame(0, 10));
    assert_eq!(buf.frame_count(), 1);
    buf.store(1, make_frame(1, 10));
    assert_eq!(buf.frame_count(), 2);
}
