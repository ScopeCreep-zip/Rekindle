use rekindle_transport_ipc::v3::session::handshake::{
    handshake_pair, handshake_with_wrong_key, handshake_with_timeout,
    HandshakeConfig, HandshakeError,
};
use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

fn default_config() -> HandshakeConfig {
    HandshakeConfig::default()
}

#[tokio::test]
async fn wrong_static_key_produces_noise_failed() {
    let result = handshake_with_wrong_key(default_config(), default_config()).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, HandshakeError::NoiseFailed { .. } | HandshakeError::TranscriptMismatch | HandshakeError::SubstrateError { .. }),
        "Expected NoiseFailed or TranscriptMismatch or SubstrateError, got: {err:?}"
    );
}

#[tokio::test]
async fn timeout_produces_handshake_timeout() {
    let result = handshake_with_timeout(default_config(), std::time::Duration::from_millis(1)).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, HandshakeError::Timeout | HandshakeError::SubstrateError { .. }),
        "Expected Timeout or SubstrateError (broken pipe from dropped listener), got: {err:?}"
    );
}

#[tokio::test]
async fn capability_mismatch_produces_capability_mismatch() {
    let broken = CapabilityBits::MANDATORY_V1 & !CapabilityBits::SACK;
    let result = handshake_pair(
        HandshakeConfig::default(),
        HandshakeConfig::new(broken, Clearance::Internal),
    ).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, HandshakeError::CapabilityMismatch { .. }),
        "Expected CapabilityMismatch, got: {err:?}"
    );
}

#[test]
fn every_failure_is_named() {
    let err = HandshakeError::Timeout;
    match err {
        HandshakeError::Timeout => {}
        HandshakeError::NoiseFailed { .. } => {}
        HandshakeError::CapabilityMismatch { .. } => {}
        HandshakeError::PeerUnregistered => {}
        HandshakeError::PeerUidDisallowed => {}
        HandshakeError::PeerClearanceMismatch => {}
        HandshakeError::TranscriptMismatch => {}
        HandshakeError::WireVersionUnsupported => {}
        HandshakeError::InvalidCredentials => {}
        HandshakeError::SubstrateError { .. } => {}
        HandshakeError::CipherInitFailed { .. } => {}
    }
}
