use rekindle_transport_ipc::v3::wire::{envelope::offsets as env, header::offsets as hdr};

#[test]
fn envelope_body_len_is_4_byte_aligned() {
    assert_eq!(env::BODY_LEN % 4, 0);
}

#[test]
fn envelope_session_seq_is_8_byte_aligned() {
    assert_eq!(env::SESSION_SEQ % 8, 0);
}

#[test]
fn header_chunk_index_is_4_byte_aligned() {
    assert_eq!(hdr::CHUNK_INDEX % 4, 0);
}

#[test]
fn header_nonce_is_8_byte_aligned() {
    assert_eq!(hdr::NONCE % 8, 0);
}

#[test]
fn envelope_mac_is_16_byte_aligned() {
    assert_eq!(env::ENVELOPE_MAC % 16, 0);
}

#[test]
fn header_mac_is_16_byte_aligned() {
    assert_eq!(hdr::HEADER_MAC % 16, 0);
}
