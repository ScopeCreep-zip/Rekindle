use rekindle_transport_ipc::v3::codec::audit::{checkpoint, query, proof, gap, replay};
use uuid::Uuid;

#[test]
fn checkpoint_roundtrip() {
    let original = checkpoint::AuditCheckpointPayload {
        chain_index: 1000,
        chain_length: 1000,
        checkpoint_seq: 5,
        wall_clock_ns: 1_700_000_000_000_000_000,
        chain_link: [0x11; 32],
        anchor_link: [0x22; 32],
    };
    let encoded = checkpoint::encode(&original);
    let decoded = checkpoint::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn query_roundtrip() {
    let original = query::AuditQueryPayload {
        query_id: Uuid::nil(),
        query_session_seq: 500,
    };
    let encoded = query::encode(&original);
    let decoded = query::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn proof_roundtrip_no_segment() {
    let original = proof::AuditProofPayload {
        query_id: Uuid::nil(),
        result_session_seq: 500,
        link: [0x33; 32],
        anchor_link: [0x44; 32],
        segment: vec![],
    };
    let encoded = proof::encode(&original);
    let decoded = proof::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn proof_roundtrip_with_segment() {
    let original = proof::AuditProofPayload {
        query_id: Uuid::nil(),
        result_session_seq: 500,
        link: [0x33; 32],
        anchor_link: [0x44; 32],
        segment: vec![[0x55; 32], [0x66; 32], [0x77; 32]],
    };
    let encoded = proof::encode(&original);
    let decoded = proof::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn gap_roundtrip() {
    let mut bitmap = vec![0u8; 2];
    bitmap[0] = 0b0010_0100; // frames 2 and 5 missing within range
    let original = gap::AuditGapPayload {
        gap_id: Uuid::nil(),
        gap_start_seq: 50,
        gap_end_seq: 60,
        gap_detected_ns: 1_700_000_000_000_000_000,
        expected_chain_link: [0x88; 32],
        missing_bitmap: bitmap,
    };
    let encoded = gap::encode(&original);
    let decoded = gap::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn gap_roundtrip_empty_bitmap() {
    let original = gap::AuditGapPayload {
        gap_id: Uuid::nil(),
        gap_start_seq: 0,
        gap_end_seq: 0,
        gap_detected_ns: 0,
        expected_chain_link: [0; 32],
        missing_bitmap: vec![],
    };
    let encoded = gap::encode(&original);
    let decoded = gap::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn replay_roundtrip() {
    let original = replay::AuditReplayPayload {
        gap_id: Uuid::nil(),
        replay_frame_count: 3,
        replay_total_bytes: 196_608,
        replayed_frames: vec![0xAA; 196_608],
    };
    let encoded = replay::encode(&original);
    let decoded = replay::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn replay_roundtrip_zero_frames() {
    let original = replay::AuditReplayPayload {
        gap_id: Uuid::nil(),
        replay_frame_count: 0,
        replay_total_bytes: 0,
        replayed_frames: vec![],
    };
    let encoded = replay::encode(&original);
    let decoded = replay::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}
