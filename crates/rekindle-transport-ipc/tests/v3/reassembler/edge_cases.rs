use rekindle_transport_ipc::v3::io::lane_channels::PlaintextBuf;
use rekindle_transport_ipc::v3::stream::reassembler::Reassembler;

fn ins(r: &mut Reassembler, idx: u32, data: &[u8]) -> Vec<(u32, PlaintextBuf)> {
    let digest = *blake3::hash(data).as_bytes();
    r.insert_with_digest(idx, PlaintextBuf::Owned(data.to_vec()), digest)
}

#[test]
fn empty_payload_stream() {
    let r = Reassembler::new(256);
    let result = r.verify_content_hash(&[0u8; 32]);
    assert!(result.is_ok());
    assert_eq!(r.total_bytes(), 0);
}

#[test]
fn single_byte_payload() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, &[42]);
    assert_eq!(r.total_bytes(), 1);
    assert_eq!(r.next_expected(), 1);

    let hash = blake3::Hasher::new()
        .update(blake3::hash(&[42]).as_bytes())
        .finalize();
    assert!(r.verify_content_hash(hash.as_bytes()).is_ok());
}

#[test]
fn fin_before_all_chunks_does_not_error() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, b"first");
    assert!(!r.is_complete(3), "must not be complete with only 1 of 3 chunks");
}

#[test]
fn reset_clears_reassembler() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, &[1, 2, 3]);
    ins(&mut r, 2, &[7, 8, 9]); // buffered, gap at 1
    assert_eq!(r.next_expected(), 1);
    assert_eq!(r.buffered_count(), 1);

    r.reset(256);

    assert_eq!(r.next_expected(), 0);
    assert_eq!(r.buffered_count(), 0);
    assert_eq!(r.total_bytes(), 0);
}

#[test]
fn reassembler_reports_total_bytes() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, &[0; 100]);
    ins(&mut r, 1, &[0; 200]);
    ins(&mut r, 2, &[0; 300]);
    assert_eq!(r.total_bytes(), 600);
}

#[test]
fn is_complete_true_when_all_chunks_received() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, &[1]);
    ins(&mut r, 1, &[2]);
    ins(&mut r, 2, &[3]);
    assert!(r.is_complete(3));
}

#[test]
fn is_complete_false_with_gap() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, &[1]);
    ins(&mut r, 2, &[3]); // gap at 1
    assert!(!r.is_complete(3));
}

#[test]
fn large_chunk_index_beyond_window_does_not_crash() {
    // Window is 256. Chunk index 255 is the last admissible.
    // Chunk index 256+ is overflow — must not crash, must return empty.
    let mut r = Reassembler::new(256);
    let d = ins(&mut r, 256, &[0xFF]);
    assert!(d.is_empty(), "chunk beyond window must be rejected");
    assert_eq!(r.buffered_count(), 0);
}
