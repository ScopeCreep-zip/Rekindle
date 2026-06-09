use rekindle_transport_ipc::v3::codec::envelope::{build_envelope, parse_envelope, EnvelopeInfo};
use rekindle_transport_ipc::v3::wire::lane::Lane;
use rekindle_transport_ipc::v3::wire::envelope::flags;

fn test_envelope_key() -> [u8; 32] {
    [0xAA; 32]
}

fn valid_info() -> EnvelopeInfo {
    EnvelopeInfo {
        wire_version: rekindle_transport_ipc::v3::wire::constants::WIRE_VERSION,
        lane: Lane::Control,
        flags: 0,
        body_len: 100,
        session_seq: 1,
    }
}

#[test]
fn valid_envelope_roundtrips() {
    let key = test_envelope_key();
    let bytes = build_envelope(&valid_info(), &key);
    let parsed = parse_envelope(&bytes, &key).expect("valid envelope must parse");
    assert_eq!(parsed.lane, Lane::Control);
    assert_eq!(parsed.body_len, 100);
    assert_eq!(parsed.session_seq, 1);
}

#[test]
fn tampered_lane_rejected_via_mac_not_lane_error() {
    let key = test_envelope_key();
    let mut bytes = build_envelope(&valid_info(), &key);
    bytes[1] = 0x02; // Control → Data
    let err = parse_envelope(&bytes, &key).unwrap_err();
    assert!(
        format!("{err:?}").contains("Mac"),
        "Must fail with MAC error, not lane-unknown. Got: {err:?}"
    );
}

#[test]
fn tampered_body_len_rejected_via_mac_not_size_error() {
    let key = test_envelope_key();
    let mut bytes = build_envelope(&valid_info(), &key);
    bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let err = parse_envelope(&bytes, &key).unwrap_err();
    assert!(
        format!("{err:?}").contains("Mac"),
        "Must fail with MAC error, not frame-too-large. Got: {err:?}"
    );
}

#[test]
fn tampered_session_seq_rejected_via_mac() {
    let key = test_envelope_key();
    let mut bytes = build_envelope(&valid_info(), &key);
    bytes[8..16].copy_from_slice(&999u64.to_le_bytes());
    let err = parse_envelope(&bytes, &key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}

#[test]
fn tampered_reserved_flags_rejected_via_mac() {
    let key = test_envelope_key();
    let mut bytes = build_envelope(&valid_info(), &key);
    // Set reserved bit 4
    let current_flags = u16::from_le_bytes([bytes[2], bytes[3]]);
    let tampered = current_flags | 0x0010;
    bytes[2..4].copy_from_slice(&tampered.to_le_bytes());
    let err = parse_envelope(&bytes, &key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}

#[test]
fn wrong_key_rejected() {
    let key = test_envelope_key();
    let wrong_key = [0xBB; 32];
    let bytes = build_envelope(&valid_info(), &key);
    let err = parse_envelope(&bytes, &wrong_key).unwrap_err();
    assert!(format!("{err:?}").contains("Mac"), "Got: {err:?}");
}

#[test]
fn valid_flags_accepted() {
    let key = test_envelope_key();
    let mut info = valid_info();
    info.flags = flags::SHUTDOWN_INITIATED | flags::BATCH_BOUNDARY;
    let bytes = build_envelope(&info, &key);
    let parsed = parse_envelope(&bytes, &key).expect("valid flags must parse");
    assert_eq!(parsed.flags, flags::SHUTDOWN_INITIATED | flags::BATCH_BOUNDARY);
}
