use rekindle_transport_ipc::v3::codec::header::{build_header, parse_header, StreamHeaderInfo};
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

fn test_header_key() -> [u8; 32] {
    [0xCC; 32]
}

fn valid_info() -> StreamHeaderInfo {
    StreamHeaderInfo {
        frame_class: FrameClass::Stream,
        frame_kind: StreamKind::Payload,
        stream_id: 5,
        header_flags: 0,
        chunk_index: 0,
        nonce: 1,
    }
}

#[test]
fn valid_header_roundtrips() {
    let key = test_header_key();
    let bytes = build_header(&valid_info(), &key);
    let parsed = parse_header(&bytes, &key).expect("valid header must parse");
    assert_eq!(parsed.stream_id, 5);
    assert_eq!(parsed.chunk_index, 0);
    assert_eq!(parsed.nonce, 1);
}

#[test]
fn tampered_stream_id_rejected_via_mac() {
    let key = test_header_key();
    let mut bytes = build_header(&valid_info(), &key);
    bytes[2] = 99;
    let err = parse_header(&bytes, &key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}

#[test]
fn tampered_frame_kind_rejected_via_mac() {
    let key = test_header_key();
    let mut bytes = build_header(&valid_info(), &key);
    bytes[1] = StreamKind::Fin as u8;
    let err = parse_header(&bytes, &key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}

#[test]
fn tampered_nonce_rejected_via_mac() {
    let key = test_header_key();
    let mut bytes = build_header(&valid_info(), &key);
    bytes[8..16].copy_from_slice(&999u64.to_le_bytes());
    let err = parse_header(&bytes, &key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}

#[test]
fn tampered_chunk_index_rejected_via_mac() {
    let key = test_header_key();
    let mut bytes = build_header(&valid_info(), &key);
    bytes[4..8].copy_from_slice(&42u32.to_le_bytes());
    let err = parse_header(&bytes, &key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}

#[test]
fn reserved_flag_bits_rejected_via_mac() {
    let key = test_header_key();
    let mut bytes = build_header(&valid_info(), &key);
    bytes[3] |= 0b0010_0000; // reserved bit 5
    let err = parse_header(&bytes, &key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}

#[test]
fn wrong_key_rejected() {
    let key = test_header_key();
    let wrong_key = [0xDD; 32];
    let bytes = build_header(&valid_info(), &key);
    let err = parse_header(&bytes, &wrong_key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}
