use super::*;

fn solid_frame(width: u32, height: u32, ts: u64) -> RawFrame {
    RawFrame {
        width,
        height,
        y: vec![0x80; RawFrame::expected_y_len(width, height)],
        u: vec![0x80; RawFrame::expected_chroma_len(width, height)],
        v: vec![0x80; RawFrame::expected_chroma_len(width, height)],
        timestamp_ms: ts,
    }
}

#[test]
fn valid_frame_flattens_to_1_5x() {
    let f = solid_frame(16, 16, 42);
    f.validate().unwrap();
    let buf = f.to_i420_contiguous().unwrap();
    // 16*16 Y + 8*8 U + 8*8 V = 256 + 64 + 64 = 384 = 16*16*3/2.
    assert_eq!(buf.len(), 384);
}

#[test]
fn odd_dimensions_rejected() {
    let mut f = solid_frame(16, 16, 0);
    f.width = 15;
    // Resize Y so only the parity check trips, not the length check.
    f.y = vec![0; 15 * 16];
    let err = f.validate().unwrap_err();
    assert!(matches!(err, VideoError::InvalidInput(_)));
}

#[test]
fn zero_dimensions_rejected() {
    let f = solid_frame(0, 0, 0);
    assert!(matches!(f.validate(), Err(VideoError::InvalidInput(_))));
}

#[test]
fn plane_size_mismatch_rejected() {
    let mut f = solid_frame(16, 16, 0);
    f.y.pop();
    assert!(matches!(f.validate(), Err(VideoError::InvalidInput(_))));
}

#[test]
fn expected_lengths_match_i420() {
    assert_eq!(RawFrame::expected_y_len(854, 480), 854 * 480);
    assert_eq!(RawFrame::expected_chroma_len(854, 480), 427 * 240);
}
