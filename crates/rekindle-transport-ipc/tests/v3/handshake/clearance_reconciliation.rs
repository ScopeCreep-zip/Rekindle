use rekindle_transport_ipc::v3::session::handshake::{handshake_pair, HandshakeConfig};
use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

fn config_with_clearance(clearance: Clearance) -> HandshakeConfig {
    HandshakeConfig::new(CapabilityBits::MANDATORY_V1, clearance)
}

#[tokio::test]
async fn agreed_clearance_is_min_of_both() {
    let (dialler, listener) = handshake_pair(
        config_with_clearance(Clearance::Confidential),
        config_with_clearance(Clearance::Internal),
    )
    .await
    .unwrap();

    assert_eq!(dialler.agreed_clearance, Clearance::Internal);
    assert_eq!(listener.agreed_clearance, Clearance::Internal);
}

#[tokio::test]
async fn same_clearance_accepted() {
    let (dialler, listener) = handshake_pair(
        config_with_clearance(Clearance::Internal),
        config_with_clearance(Clearance::Internal),
    )
    .await
    .unwrap();

    assert_eq!(dialler.agreed_clearance, Clearance::Internal);
    assert_eq!(listener.agreed_clearance, Clearance::Internal);
}

#[tokio::test]
async fn dialler_exceeds_allowance_gets_downgraded() {
    let (dialler, listener) = handshake_pair(
        config_with_clearance(Clearance::Secret),
        config_with_clearance(Clearance::Public),
    )
    .await
    .unwrap();

    assert_eq!(dialler.agreed_clearance, Clearance::Public);
    assert_eq!(listener.agreed_clearance, Clearance::Public);
}

#[tokio::test]
async fn listener_exceeds_dialler_gets_downgraded() {
    let (dialler, listener) = handshake_pair(
        config_with_clearance(Clearance::Public),
        config_with_clearance(Clearance::TopSecret),
    )
    .await
    .unwrap();

    assert_eq!(dialler.agreed_clearance, Clearance::Public);
    assert_eq!(listener.agreed_clearance, Clearance::Public);
}

#[tokio::test]
async fn unclassified_is_valid_minimum() {
    let (dialler, _) = handshake_pair(
        config_with_clearance(Clearance::Unclassified),
        config_with_clearance(Clearance::TopSecret),
    )
    .await
    .unwrap();

    assert_eq!(dialler.agreed_clearance, Clearance::Unclassified);
}
