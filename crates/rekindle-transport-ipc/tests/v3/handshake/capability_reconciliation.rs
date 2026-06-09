use rekindle_transport_ipc::v3::session::handshake::{
    handshake_pair, HandshakeConfig,
    AEAD_CODE_AES256GCM, AEAD_CODE_AEGIS128L, AEAD_CODE_AEGIS128X2,
};
use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

fn config_with_caps(caps: CapabilityBits) -> HandshakeConfig {
    HandshakeConfig::new(caps, Clearance::Internal)
}

#[tokio::test]
async fn intersection_of_capabilities() {
    let dialler_caps = CapabilityBits::MANDATORY_V1
        | CapabilityBits::HANDOFF_MEMFD
        | CapabilityBits::DEDUP_CACHE;
    let listener_caps = CapabilityBits::MANDATORY_V1
        | CapabilityBits::HANDOFF_MEMFD;

    let (dialler, _listener) =
        handshake_pair(config_with_caps(dialler_caps), config_with_caps(listener_caps)).await.unwrap();

    assert!(dialler.active_capabilities.contains(CapabilityBits::HANDOFF_MEMFD));
    assert!(!dialler.active_capabilities.contains(CapabilityBits::DEDUP_CACHE));
}

#[tokio::test]
async fn mandatory_bits_missing_aborts() {
    let broken = CapabilityBits::MANDATORY_V1 & !CapabilityBits::AUDIT_CHAIN;

    let result = handshake_pair(
        config_with_caps(CapabilityBits::MANDATORY_V1),
        config_with_caps(broken),
    ).await;

    assert!(result.is_err());
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("Capability") || err.contains("capability"),
        "Expected capability mismatch error, got: {err}"
    );
}

#[tokio::test]
async fn optional_bits_negotiated_correctly() {
    let with_quiescence = CapabilityBits::MANDATORY_V1 | CapabilityBits::QUIESCENCE;
    let without_quiescence = CapabilityBits::MANDATORY_V1;

    let (dialler, _) =
        handshake_pair(config_with_caps(with_quiescence), config_with_caps(without_quiescence))
            .await
            .unwrap();

    assert!(!dialler.active_capabilities.contains(CapabilityBits::QUIESCENCE));
}

#[tokio::test]
async fn both_have_optional_bit_is_active() {
    let both = CapabilityBits::MANDATORY_V1 | CapabilityBits::QUIESCENCE;

    let (dialler, _) =
        handshake_pair(config_with_caps(both), config_with_caps(both)).await.unwrap();

    assert!(dialler.active_capabilities.contains(CapabilityBits::QUIESCENCE));
}

/// When both sides advertise AEGIS capability, the negotiated AEAD
/// must be an AEGIS variant (128L or 128X2), not GCM. Which variant
/// is selected depends on a runtime probe cached per-process via
/// OnceLock. On CPUs where 128L and X2 perform within measurement
/// noise of each other, an independent probe in the same binary may
/// get the opposite result due to the OnceLock already being set.
/// The test asserts the contract (AEGIS selected, both sides agree),
/// not which specific variant the probe picks.
#[tokio::test]
async fn negotiated_aead_is_aegis_when_capability_set() {
    let caps = CapabilityBits::MANDATORY_V1 | CapabilityBits::AEAD_AEGIS128L;
    let (dialler, listener) = handshake_pair(
        config_with_caps(caps), config_with_caps(caps),
    ).await.unwrap();

    // Both sides must agree on the same algorithm.
    assert_eq!(dialler.agreed_aead, listener.agreed_aead,
        "dialler and listener must agree on AEAD algorithm");

    // The agreed algorithm must be one of the AEGIS variants (not GCM)
    // since both sides advertised AEAD_AEGIS128L capability.
    assert!(
        dialler.agreed_aead == AEAD_CODE_AEGIS128L || dialler.agreed_aead == AEAD_CODE_AEGIS128X2,
        "with AEGIS capability, agreed_aead must be 128L or 128X2, got 0x{:02x}",
        dialler.agreed_aead
    );
}

/// FIPS mode forces AES-256-GCM regardless of AEGIS availability.
#[tokio::test]
async fn fips_mode_forces_aes256gcm() {
    let caps = CapabilityBits::MANDATORY_V1
        | CapabilityBits::AEAD_AEGIS128L
        | CapabilityBits::FIPS_MODE;
    let (dialler, _) = handshake_pair(
        config_with_caps(caps), config_with_caps(caps),
    ).await.unwrap();

    assert_eq!(dialler.agreed_aead, AEAD_CODE_AES256GCM,
        "FIPS mode must force AES-256-GCM");
}

/// Without AEGIS capability, falls back to AES-256-GCM.
#[tokio::test]
async fn no_aegis_falls_back_to_gcm() {
    let caps = CapabilityBits::MANDATORY_V1; // no AEAD_AEGIS128L
    let (dialler, _) = handshake_pair(
        config_with_caps(caps), config_with_caps(caps),
    ).await.unwrap();

    assert_eq!(dialler.agreed_aead, AEAD_CODE_AES256GCM,
        "without AEGIS capability, must fall back to AES-256-GCM");
}

#[tokio::test]
async fn unknown_bits_ignored() {
    let with_unknown = CapabilityBits::MANDATORY_V1 | CapabilityBits::from_bits_retain(1u64 << 50);
    let normal = CapabilityBits::MANDATORY_V1;

    let result = handshake_pair(config_with_caps(with_unknown), config_with_caps(normal)).await;
    assert!(result.is_ok(), "Unknown bits in peer's advertisement must not cause failure");
}
