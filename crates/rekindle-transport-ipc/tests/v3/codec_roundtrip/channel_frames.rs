use rekindle_transport_ipc::v3::codec::channel::{hello, hello_ack, ping, nack, credit, goodbye};
use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::failure::FailureCode;
use uuid::Uuid;

#[test]
fn hello_roundtrip() {
    let original = hello::HelloPayload {
        session_id_proposal: Uuid::nil(),
        dialler_peer_id: [0xAA; 32],
        capabilities: CapabilityBits::MANDATORY_V1,
        proposed_clearance: Clearance::Internal,
        dialler_epoch_ns: 1_700_000_000_000_000_000,
        dialler_handshake_hash: [0xBB; 32],
    };
    let encoded = hello::encode(&original);
    let decoded = hello::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn hello_ack_roundtrip() {
    let original = hello_ack::HelloAckPayload {
        session_id: Uuid::nil(),
        listener_peer_id: [0xCC; 32],
        capabilities: CapabilityBits::MANDATORY_V1,
        agreed_clearance: Clearance::Confidential,
        agreed_aead: 0x01, // AES-256-GCM
        listener_epoch_ns: 1_700_000_000_000_000_000,
        listener_handshake_hash: [0xDD; 32],
    };
    let encoded = hello_ack::encode(&original);
    let decoded = hello_ack::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn ping_roundtrip() {
    let original = ping::PingPayload {
        ping_nonce: 0xDEAD_BEEF_CAFE_BABE,
        sender_epoch_ns: 1_700_000_000_000_000_000,
        last_seen_remote_seq: 42,
    };
    let encoded = ping::encode(&original);
    let decoded = ping::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn nack_roundtrip() {
    let original = nack::NackPayload {
        reason_code: FailureCode::ReplayDetected,
        rejected_message_id: Uuid::nil(),
        session_seq_rejected: 99,
        detail: "test nack".into(),
    };
    let encoded = nack::encode(&original);
    let decoded = nack::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn nack_roundtrip_empty_detail() {
    let original = nack::NackPayload {
        reason_code: FailureCode::FrameMalformed,
        rejected_message_id: Uuid::nil(),
        session_seq_rejected: 0,
        detail: String::new(),
    };
    let encoded = nack::encode(&original);
    let decoded = nack::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn credit_roundtrip() {
    let original = credit::CreditPayload {
        scope: 0x02, // Stream-scoped
        lane_or_stream_id: 7,
        credit_bytes: 1_048_576,
        credit_frames: 64,
        credit_generation: 3,
    };
    let encoded = credit::encode(&original);
    let decoded = credit::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn goodbye_roundtrip() {
    let original = goodbye::GoodbyePayload {
        reason_code: 0,
        drain_timeout_ms: 30_000,
        final_session_seq: 10_000,
    };
    let encoded = goodbye::encode(&original);
    let decoded = goodbye::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}
