use rekindle_transport_ipc::v3::wire::header::StreamHeaderWire;

#[test]
fn stream_header_is_exactly_32_bytes() {
    assert_eq!(
        std::mem::size_of::<StreamHeaderWire>(),
        32,
        "StreamHeaderWire must be exactly 32 bytes. \
         Any change is a wire-incompatible break."
    );
}
