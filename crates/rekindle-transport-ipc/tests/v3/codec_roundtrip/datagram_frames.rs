use rekindle_transport_ipc::v3::codec::datagram::{request, reply, notify, publish, reject};
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::failure::FailureCode;
use uuid::Uuid;

#[test]
fn request_roundtrip() {
    let original = request::DatagramRequestPayload {
        message_id: Uuid::nil(),
        reply_timeout_ms: 5000,
        sender_clearance: Clearance::Internal,
        application_payload: vec![1, 2, 3, 4],
        conditions: vec![],
    };
    let encoded = request::encode(&original);
    let decoded = request::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn request_roundtrip_with_conditions() {
    let original = request::DatagramRequestPayload {
        message_id: Uuid::nil(),
        reply_timeout_ms: 1000,
        sender_clearance: Clearance::Confidential,
        application_payload: vec![0xFF; 200],
        conditions: vec![0x01, 0x02, 0x03],
    };
    let encoded = request::encode(&original);
    let decoded = request::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn reply_roundtrip() {
    let original = reply::DatagramReplyPayload {
        message_id: Uuid::nil(),
        correlation_id: Uuid::nil(),
        status_phase: 0x03, // Succeeded
        application_payload: vec![10, 20, 30],
        status_reason: "ok".into(),
        status_message: "operation completed".into(),
        conditions: vec![],
    };
    let encoded = reply::encode(&original);
    let decoded = reply::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn reply_roundtrip_failed_with_detail() {
    let original = reply::DatagramReplyPayload {
        message_id: Uuid::nil(),
        correlation_id: Uuid::nil(),
        status_phase: 0x04, // Failed
        application_payload: vec![],
        status_reason: "vault-not-open".into(),
        status_message: "The vault must be unlocked first".into(),
        conditions: vec![],
    };
    let encoded = reply::encode(&original);
    let decoded = reply::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn notify_roundtrip() {
    let original = notify::DatagramNotifyPayload {
        message_id: Uuid::nil(),
        sender_clearance: Clearance::Internal,
        application_payload: vec![42],
    };
    let encoded = notify::encode(&original);
    let decoded = notify::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn publish_roundtrip() {
    let original = publish::DatagramPublishPayload {
        message_id: Uuid::nil(),
        topic_hash: [0xAB; 32],
        event_seq: 7,
        event_timestamp_ns: 1_700_000_000_000_000_000,
        sender_clearance: Clearance::Public,
        application_payload: vec![0xFF; 200],
        conditions: vec![],
    };
    let encoded = publish::encode(&original);
    let decoded = publish::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}

#[test]
fn reject_roundtrip() {
    let original = reject::DatagramRejectPayload {
        reason_code: FailureCode::ClearanceInsufficient,
        rejected_message_id: Uuid::nil(),
        detail: "clearance too low".into(),
    };
    let encoded = reject::encode(&original);
    let decoded = reject::decode(&encoded).expect("decode failed");
    assert_eq!(original, decoded);
}
