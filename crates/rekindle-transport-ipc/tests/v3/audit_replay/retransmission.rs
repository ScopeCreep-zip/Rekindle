use rekindle_transport_ipc::v3::audit::retention::{RetentionBuffer, RetentionConfig};
use rekindle_transport_ipc::v3::audit::gap::MissingBitmapBuilder;
use rekindle_transport_ipc::v3::audit::replay::{build_replay, ReplayResult};

fn make_frame(seq: u64) -> Vec<u8> {
    let mut frame = vec![0u8; 64];
    frame[0..8].copy_from_slice(&seq.to_le_bytes());
    frame[8] = 0xFF; // marker byte
    frame
}

fn populated_buffer(range: std::ops::Range<u64>) -> RetentionBuffer {
    let config = RetentionConfig { max_frames: 1000, max_bytes: 1_048_576 };
    let mut buf = RetentionBuffer::new(config);
    for seq in range {
        buf.store(seq, make_frame(seq));
    }
    buf
}

#[test]
fn replay_contains_only_requested_frames() {
    let buf = populated_buffer(0..20);
    let mut bm = MissingBitmapBuilder::new(5, 15);
    bm.mark_missing(7);
    bm.mark_missing(12);
    let bitmap = bm.build();

    let result = build_replay(&buf, 5, 15, &bitmap);
    match result {
        ReplayResult::Frames(frames) => {
            assert_eq!(frames.len(), 2, "only 2 frames requested");
            // Verify seq numbers
            let seq0 = u64::from_le_bytes(frames[0][0..8].try_into().unwrap());
            let seq1 = u64::from_le_bytes(frames[1][0..8].try_into().unwrap());
            assert_eq!(seq0, 7);
            assert_eq!(seq1, 12);
        }
        other => panic!("Expected Frames, got {other:?}"),
    }
}

#[test]
fn replayed_frames_preserve_original_bytes() {
    let buf = populated_buffer(0..10);
    let mut bm = MissingBitmapBuilder::new(0, 9);
    bm.mark_missing(3);
    let bitmap = bm.build();

    let result = build_replay(&buf, 0, 9, &bitmap);
    match result {
        ReplayResult::Frames(frames) => {
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0], make_frame(3));
        }
        other => panic!("Expected Frames, got {other:?}"),
    }
}

#[test]
fn replay_with_zero_missing_frames() {
    let buf = populated_buffer(0..10);
    let bm = MissingBitmapBuilder::new(0, 9); // none marked missing
    let bitmap = bm.build();

    let result = build_replay(&buf, 0, 9, &bitmap);
    match result {
        ReplayResult::Frames(frames) => {
            assert_eq!(frames.len(), 0);
        }
        other => panic!("Expected empty Frames, got {other:?}"),
    }
}

#[test]
fn unfillable_gap_returns_unfillable() {
    let config = RetentionConfig { max_frames: 5, max_bytes: 1_048_576 };
    let mut buf = RetentionBuffer::new(config);
    // Only store frames 10..15, not 0..10
    for seq in 10..15 {
        buf.store(seq, make_frame(seq));
    }

    let mut bm = MissingBitmapBuilder::new(0, 9);
    bm.mark_missing(3); // frame 3 was never stored
    let bitmap = bm.build();

    let result = build_replay(&buf, 0, 9, &bitmap);
    assert!(
        matches!(result, ReplayResult::Unfillable { .. }),
        "Must return Unfillable when requested frames are not in retention, got {result:?}"
    );
}

#[test]
fn replay_multiple_non_contiguous() {
    let buf = populated_buffer(0..100);
    let mut bm = MissingBitmapBuilder::new(10, 50);
    bm.mark_missing(12);
    bm.mark_missing(25);
    bm.mark_missing(37);
    bm.mark_missing(49);
    let bitmap = bm.build();

    let result = build_replay(&buf, 10, 50, &bitmap);
    match result {
        ReplayResult::Frames(frames) => {
            assert_eq!(frames.len(), 4);
            let seqs: Vec<u64> = frames
                .iter()
                .map(|f| u64::from_le_bytes(f[0..8].try_into().unwrap()))
                .collect();
            assert_eq!(seqs, vec![12, 25, 37, 49]);
        }
        other => panic!("Expected 4 Frames, got {other:?}"),
    }
}
