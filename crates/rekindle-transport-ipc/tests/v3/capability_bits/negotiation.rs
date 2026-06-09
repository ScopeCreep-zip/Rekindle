use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;

#[test]
fn negotiation_is_bitwise_and() {
    let peer_a = CapabilityBits::MANDATORY_V1
        | CapabilityBits::HANDOFF_MEMFD
        | CapabilityBits::DEDUP_CACHE;
    let peer_b = CapabilityBits::MANDATORY_V1
        | CapabilityBits::HANDOFF_MEMFD;

    let active = peer_a & peer_b;

    assert!(active.contains(CapabilityBits::HANDOFF_MEMFD));
    assert!(!active.contains(CapabilityBits::DEDUP_CACHE));
}

#[test]
fn mandatory_survives_negotiation_with_mandatory() {
    let active = CapabilityBits::MANDATORY_V1 & CapabilityBits::MANDATORY_V1;
    assert_eq!(active, CapabilityBits::MANDATORY_V1);
}

#[test]
fn mandatory_survives_negotiation_with_superset() {
    let superset = CapabilityBits::MANDATORY_V1 | CapabilityBits::QUIESCENCE;
    let active = CapabilityBits::MANDATORY_V1 & superset;
    assert!(active.contains(CapabilityBits::MANDATORY_V1));
}

#[test]
fn empty_peer_kills_all_capabilities() {
    let active = CapabilityBits::MANDATORY_V1 & CapabilityBits::empty();
    assert!(active.is_empty());
}
