use rekindle_transport_ipc::v3::codec::handoff::{offer, accept, reject, revoke, confirm};
use rekindle_transport_ipc::v3::wire::failure::FailureCode;
use uuid::Uuid;

#[test]
fn offer_roundtrip() {
    let original = offer::HandoffOfferPayload {
        handoff_id: Uuid::nil(),
        fd_kind: 0x01, // memfd
        fd_sealed: true,
        stream_id: 5,
        offer_timeout_ms: 5000,
        payload_size_bytes: 104_857_600,
        content_hash: [0x44; 32],
    };
    let encoded = offer::encode(&original);
    let decoded = offer::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn accept_roundtrip() {
    let original = accept::HandoffAcceptPayload {
        handoff_id: Uuid::nil(),
        accept_wall_ns: 1_700_000_000_000_000_000,
        verified_content_hash: [0x55; 32],
    };
    let encoded = accept::encode(&original);
    let decoded = accept::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn reject_roundtrip() {
    let original = reject::HandoffRejectPayload {
        reason_code: FailureCode::HandoffContentHashMismatch,
        handoff_id: Uuid::nil(),
        detail: "hash mismatch after mmap".into(),
    };
    let encoded = reject::encode(&original);
    let decoded = reject::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn revoke_roundtrip() {
    let original = revoke::HandoffRevokePayload {
        reason_code: FailureCode::HandoffTimeout,
        handoff_id: Uuid::nil(),
    };
    let encoded = revoke::encode(&original);
    let decoded = revoke::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn confirm_roundtrip() {
    let original = confirm::HandoffConfirmPayload {
        handoff_id: Uuid::nil(),
    };
    let encoded = confirm::encode(&original);
    let decoded = confirm::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}
