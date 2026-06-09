use rekindle_transport_ipc::v3::wire::envelope::EnvelopeWire;

#[test]
fn envelope_is_exactly_32_bytes() {
    assert_eq!(
        std::mem::size_of::<EnvelopeWire>(),
        32,
        "EnvelopeWire must be exactly 32 bytes. \
         Any change is a wire-incompatible break."
    );
}
