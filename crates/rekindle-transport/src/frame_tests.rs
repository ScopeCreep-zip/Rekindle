use super::*;

#[test]
fn roundtrip_encode_decode() {
    let payload = b"hello, rekindle";
    let frame = encode(TypeId::CommunityLeave, payload).unwrap();
    let (tid, decoded) = decode(&frame).unwrap();
    assert_eq!(tid, TypeId::CommunityLeave);
    assert_eq!(decoded, payload);
}

#[test]
fn rejects_unknown_version() {
    let mut frame = encode(TypeId::CommunityLeave, b"test").unwrap();
    frame[0] = 0xFF;
    assert!(matches!(
        decode(&frame),
        Err(TransportError::UnknownVersion { version: 0xFF })
    ));
}

#[test]
fn rejects_unknown_type() {
    let mut frame = encode(TypeId::CommunityLeave, b"test").unwrap();
    frame[1] = 0xFE;
    assert!(matches!(
        decode(&frame),
        Err(TransportError::UnknownType { type_id: 0xFE })
    ));
}

#[test]
fn rejects_too_short() {
    assert!(matches!(
        decode(&[0x01, 0x01]),
        Err(TransportError::InvalidFrame { .. })
    ));
}

#[test]
fn rejects_payload_too_large() {
    let big = vec![0u8; MAX_PAYLOAD_SIZE + 1];
    assert!(matches!(
        encode(TypeId::CommunityLeave, &big),
        Err(TransportError::PayloadTooLarge { .. })
    ));
}

#[test]
fn empty_payload_roundtrip() {
    let frame = encode(TypeId::CommunityLeave, &[]).unwrap();
    let (tid, payload) = decode(&frame).unwrap();
    assert_eq!(tid, TypeId::CommunityLeave);
    assert!(payload.is_empty());
}

#[test]
fn max_payload_roundtrip() {
    let payload = vec![0xAB; MAX_PAYLOAD_SIZE];
    let frame = encode(TypeId::GossipBroadcast, &payload).unwrap();
    let (tid, decoded) = decode(&frame).unwrap();
    assert_eq!(tid, TypeId::GossipBroadcast);
    assert_eq!(decoded.len(), MAX_PAYLOAD_SIZE);
}

#[test]
fn all_type_ids_have_stable_byte_values() {
    assert_eq!(TypeId::GossipBroadcast as u8, 0x10);
    assert_eq!(TypeId::CommunityLeave as u8, 0x21);
    assert_eq!(TypeId::from_byte(0x23), None);
    assert_eq!(TypeId::from_byte(0x24), None);
}

#[test]
fn from_byte_roundtrip_all_known() {
    for byte in 0..=0xFF_u8 {
        if let Some(tid) = TypeId::from_byte(byte) {
            assert_eq!(tid as u8, byte);
        }
    }
}
