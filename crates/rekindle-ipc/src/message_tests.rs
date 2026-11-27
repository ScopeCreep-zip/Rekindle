use super::*;

fn test_ctx() -> MessageContext {
    MessageContext::new(Uuid::nil())
}

#[test]
fn message_new_sets_wire_version() {
    let msg = Message::new(&test_ctx(), "hello", SecurityLevel::Open, Instant::now());
    assert_eq!(msg.wire_version, WIRE_VERSION);
}

#[test]
fn message_new_defaults_are_safe() {
    let msg = Message::new(&test_ctx(), 42u32, SecurityLevel::Open, Instant::now());
    // [RC-2] verified_sender_name must be None on construction.
    assert!(msg.verified_sender_name.is_none());
    // correlation_id must be None on new requests.
    assert!(msg.correlation_id.is_none());
}

#[test]
fn message_roundtrip_preserves_wire_version() {
    let msg = Message::new(
        &test_ctx(),
        "test",
        SecurityLevel::Authenticated,
        Instant::now(),
    );
    let bytes = super::super::framing::encode_frame(&msg).unwrap();
    let decoded: Message<String> = super::super::framing::decode_frame(&bytes).unwrap();
    assert_eq!(decoded.wire_version, WIRE_VERSION);
    assert_eq!(decoded.payload, "test");
}

#[test]
fn with_correlation_sets_id() {
    let msg = Message::new(&test_ctx(), 0u8, SecurityLevel::Open, Instant::now());
    let corr = Uuid::now_v7();
    let msg = msg.with_correlation(corr);
    assert_eq!(msg.correlation_id, Some(corr));
}

#[test]
fn security_level_ordering() {
    assert!(SecurityLevel::Open < SecurityLevel::Authenticated);
    assert!(SecurityLevel::Authenticated < SecurityLevel::Agent);
    assert!(SecurityLevel::Agent < SecurityLevel::Internal);
    assert!(SecurityLevel::Internal < SecurityLevel::Admin);
}
