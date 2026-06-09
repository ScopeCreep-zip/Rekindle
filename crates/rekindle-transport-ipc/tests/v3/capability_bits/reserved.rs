use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;

#[test]
fn mandatory_v1_has_no_reserved_bits_set() {
    let reserved = CapabilityBits::MANDATORY_V1.bits() & CapabilityBits::RESERVED_MASK;
    assert_eq!(
        reserved, 0,
        "MANDATORY_V1 must not set any reserved bits (34..63). \
         Found: 0b{reserved:064b}"
    );
}

#[test]
fn reserved_mask_covers_bits_34_through_63() {
    // Bits 0..33 should be zero in RESERVED_MASK.
    // Bits 34..63 should be one.
    for bit in 0..34 {
        assert_eq!(
            CapabilityBits::RESERVED_MASK & (1u64 << bit),
            0,
            "RESERVED_MASK must not cover bit {bit}"
        );
    }
    for bit in 34..64 {
        assert_ne!(
            CapabilityBits::RESERVED_MASK & (1u64 << bit),
            0,
            "RESERVED_MASK must cover bit {bit}"
        );
    }
}
