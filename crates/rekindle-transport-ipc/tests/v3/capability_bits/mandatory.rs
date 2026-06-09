use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;

#[test]
fn mandatory_v1_includes_aes256gcm() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::AEAD_AES256GCM));
}

#[test]
fn mandatory_v1_includes_blake3() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::HASH_BLAKE3));
}

#[test]
fn mandatory_v1_includes_lane_control() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::LANE_CONTROL));
}

#[test]
fn mandatory_v1_includes_lane_data() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::LANE_DATA));
}

#[test]
fn mandatory_v1_includes_audit_chain() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::AUDIT_CHAIN));
}

#[test]
fn mandatory_v1_includes_flow_credit() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::FLOW_CREDIT));
}

#[test]
fn mandatory_v1_includes_batched_ack() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::BATCHED_ACK));
}

#[test]
fn mandatory_v1_includes_key_rotation() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::KEY_ROTATION));
}

#[test]
fn mandatory_v1_includes_sack() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::SACK));
}

#[test]
fn mandatory_v1_includes_sidechannel_credit() {
    assert!(CapabilityBits::MANDATORY_V1.contains(CapabilityBits::SIDECHANNEL_CREDIT));
}
