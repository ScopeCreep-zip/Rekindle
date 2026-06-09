use rekindle_transport_ipc::v3::codec::stream::{
    open, fin, ack, nack, sack, resume, resume_deny, reference, credit,
    cancel, cancel_ack, reset, fault,
};
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::failure::FailureCode;
use uuid::Uuid;

#[test]
fn open_roundtrip() {
    let original = open::StreamOpenPayload {
        transfer_id: Uuid::nil(),
        expected_total_bytes: 16_777_216,
        expected_chunk_count: 1,
        chunk_size: 16_777_216,
        content_hash: [0xFF; 32],
        lineage_kind: 0x01,
        dedup_hint: 0x02,
        clearance_required: Clearance::Internal,
        conditions: vec![],
    };
    let encoded = open::encode(&original);
    let decoded = open::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn open_roundtrip_with_conditions() {
    let original = open::StreamOpenPayload {
        transfer_id: Uuid::nil(),
        expected_total_bytes: 0,
        expected_chunk_count: 0,
        chunk_size: 65536,
        content_hash: [0x00; 32],
        lineage_kind: 0x01,
        dedup_hint: 0x03,
        clearance_required: Clearance::Public,
        conditions: vec![0xAA, 0xBB, 0xCC, 0xDD],
    };
    let encoded = open::encode(&original);
    let decoded = open::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn fin_roundtrip() {
    let original = fin::StreamFinPayload {
        total_bytes: 1_048_576,
        fault_count: 0,
        final_content_hash: [0xEE; 32],
        final_audit_link: [0xDD; 32],
    };
    let encoded = fin::encode(&original);
    let decoded = fin::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn ack_roundtrip() {
    let original = ack::StreamAckPayload {
        transfer_id: Uuid::nil(),
        ack_byte_count: 67_108_864,
        ack_chunk_count: 4,
        audit_link: [0xAA; 32],
    };
    let encoded = ack::encode(&original);
    let decoded = ack::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn nack_roundtrip() {
    let original = nack::StreamNackPayload {
        reason_code: FailureCode::ContentHashMismatch,
        rejected_chunk_idx: 42,
        detail: "integrity failure".into(),
    };
    let encoded = nack::encode(&original);
    let decoded = nack::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn sack_roundtrip() {
    let mut bitmap = vec![0u8; 128];
    bitmap[0] = 0b1010_1010;
    bitmap[1] = 0xFF;
    let original = sack::StreamSackPayload {
        transfer_id: Uuid::nil(),
        cumulative_through: 100,
        sack_bitmap: bitmap,
    };
    let encoded = sack::encode(&original);
    let decoded = sack::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn sack_roundtrip_empty_bitmap() {
    let original = sack::StreamSackPayload {
        transfer_id: Uuid::nil(),
        cumulative_through: 0,
        sack_bitmap: vec![],
    };
    let encoded = sack::encode(&original);
    let decoded = sack::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn resume_roundtrip() {
    let original = resume::StreamResumePayload {
        transfer_id: Uuid::nil(),
        resume_from_byte: 67_108_864,
        resume_from_chunk: 4,
        anchor_audit_link: [0xAA; 32],
        anchor_content_hash: [0xBB; 32],
    };
    let encoded = resume::encode(&original);
    let decoded = resume::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn resume_deny_roundtrip() {
    let original = resume_deny::StreamResumeDenyPayload {
        transfer_id: Uuid::nil(),
        reason_code: FailureCode::ResumeWindowExpired,
    };
    let encoded = resume_deny::encode(&original);
    let decoded = resume_deny::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn reference_roundtrip_full() {
    let original = reference::StreamReferencePayload {
        transfer_id: Uuid::nil(),
        content_hash: [0xCC; 32],
        expected_total_bytes: 1_000_000,
        expected_chunk_count: 1,
        reference_kind: 0x01,
        sender_clearance: Clearance::Confidential,
        prefix_byte_count: 0,
        prefix_content_hash: [0; 32],
    };
    let encoded = reference::encode(&original);
    let decoded = reference::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn reference_roundtrip_partial_prefix() {
    let original = reference::StreamReferencePayload {
        transfer_id: Uuid::nil(),
        content_hash: [0xCC; 32],
        expected_total_bytes: 1_000_000,
        expected_chunk_count: 10,
        reference_kind: 0x02,
        sender_clearance: Clearance::Internal,
        prefix_byte_count: 500_000,
        prefix_content_hash: [0xDD; 32],
    };
    let encoded = reference::encode(&original);
    let decoded = reference::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn credit_roundtrip() {
    let original = credit::StreamCreditPayload {
        bytes: 1_048_576,
        chunks: 64,
        generation: 5,
    };
    let encoded = credit::encode(&original);
    let decoded = credit::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn cancel_roundtrip() {
    let original = cancel::StreamCancelPayload {
        transfer_id: Uuid::nil(),
        bytes_through: 500_000,
        chunks_through: 3,
    };
    let encoded = cancel::encode(&original);
    let decoded = cancel::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn cancel_ack_roundtrip() {
    let original = cancel_ack::StreamCancelAckPayload {
        transfer_id: Uuid::nil(),
        bytes_through: 500_000,
        chunks_through: 3,
    };
    let encoded = cancel_ack::encode(&original);
    let decoded = cancel_ack::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn reset_roundtrip() {
    let original = reset::StreamResetPayload {
        reason_code: FailureCode::PoolExhausted,
    };
    let encoded = reset::encode(&original);
    let decoded = reset::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn fault_roundtrip() {
    let original = fault::StreamFaultPayload {
        fault_code: 0x0042,
        fault_payload: vec![1, 2, 3, 4, 5],
    };
    let encoded = fault::encode(&original);
    let decoded = fault::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn fault_roundtrip_empty_payload() {
    let original = fault::StreamFaultPayload {
        fault_code: 0,
        fault_payload: vec![],
    };
    let encoded = fault::encode(&original);
    let decoded = fault::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}
