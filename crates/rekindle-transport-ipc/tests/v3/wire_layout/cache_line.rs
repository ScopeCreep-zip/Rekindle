use rekindle_transport_ipc::v3::wire::{envelope::EnvelopeWire, header::StreamHeaderWire};

#[test]
fn envelope_plus_header_fits_one_cache_line() {
    let combined = std::mem::size_of::<EnvelopeWire>()
        + std::mem::size_of::<StreamHeaderWire>();
    assert_eq!(
        combined, 64,
        "Envelope ({}) + StreamHeader ({}) = {} bytes, must be 64 (one L1 cache line).",
        std::mem::size_of::<EnvelopeWire>(),
        std::mem::size_of::<StreamHeaderWire>(),
        combined,
    );
}
