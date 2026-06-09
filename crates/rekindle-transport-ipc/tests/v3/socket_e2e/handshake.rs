//! Handshake tests over real Unix sockets.

use super::harness::*;

#[tokio::test]
async fn handshake_completes_over_real_socket() {
    let f = connected_pair().await;
    assert!(!f.session_id().is_nil());
    assert_eq!(f.router.requests.lock().len(), 0, "no requests yet — just handshake");
}

#[tokio::test]
async fn wrong_key_handshake_rejected() {
    let tempdir = tempfile::tempdir().unwrap();
    let socket_path = tempdir.path().join("wrong-key.sock");

    let server_keypair = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
    let client_keypair = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
    let wrong_keypair = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();

    let server = IpcFixture::bind_server(
        &socket_path,
        server_keypair,
        IpcFixtureConfig::for_test(),
    ).await;

    // Client uses wrong server public key — handshake must fail
    let wrong_pub: [u8; 32] = wrong_keypair.public.try_into().unwrap();
    let result = rekindle_transport_ipc::v3::client::IpcClient::connect(
        &socket_path, &wrong_pub, &client_keypair,
        IpcFixtureConfig::for_test().to_session_config(), handshake_config(),
    ).await;

    assert!(result.is_err(), "handshake must fail with wrong server key");
    assert_eq!(server.router.requests.lock().len(), 0, "failed handshake must produce zero deliveries");
}
