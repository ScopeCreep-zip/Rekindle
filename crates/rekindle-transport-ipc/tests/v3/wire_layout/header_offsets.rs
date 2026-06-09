use rekindle_transport_ipc::v3::wire::header::offsets;

#[test]
fn frame_class_at_offset_0() {
    assert_eq!(offsets::FRAME_CLASS, 0);
}

#[test]
fn frame_kind_at_offset_1() {
    assert_eq!(offsets::FRAME_KIND, 1);
}

#[test]
fn stream_id_at_offset_2() {
    assert_eq!(offsets::STREAM_ID, 2);
}

#[test]
fn header_flags_at_offset_3() {
    assert_eq!(offsets::HEADER_FLAGS, 3);
}

#[test]
fn chunk_index_at_offset_4() {
    assert_eq!(offsets::CHUNK_INDEX, 4);
}

#[test]
fn nonce_at_offset_8() {
    assert_eq!(offsets::NONCE, 8);
}

#[test]
fn header_mac_at_offset_16() {
    assert_eq!(offsets::HEADER_MAC, 16);
}

#[test]
fn mac_input_covers_first_16_bytes() {
    assert_eq!(offsets::MAC_INPUT_LEN, 16);
    assert_eq!(offsets::MAC_INPUT_LEN, offsets::HEADER_MAC);
}
