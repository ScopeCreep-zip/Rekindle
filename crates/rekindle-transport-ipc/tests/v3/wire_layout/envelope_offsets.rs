use rekindle_transport_ipc::v3::wire::envelope::offsets;

#[test]
fn wire_version_at_offset_0() {
    assert_eq!(offsets::WIRE_VERSION, 0);
}

#[test]
fn lane_at_offset_1() {
    assert_eq!(offsets::LANE, 1);
}

#[test]
fn flags_at_offset_2() {
    assert_eq!(offsets::FLAGS, 2);
}

#[test]
fn body_len_at_offset_4() {
    assert_eq!(offsets::BODY_LEN, 4);
}

#[test]
fn session_seq_at_offset_8() {
    assert_eq!(offsets::SESSION_SEQ, 8);
}

#[test]
fn envelope_mac_at_offset_16() {
    assert_eq!(offsets::ENVELOPE_MAC, 16);
}

#[test]
fn emac_input_covers_first_16_bytes() {
    assert_eq!(offsets::EMAC_INPUT_LEN, 16);
    assert_eq!(offsets::EMAC_INPUT_LEN, offsets::ENVELOPE_MAC);
}
