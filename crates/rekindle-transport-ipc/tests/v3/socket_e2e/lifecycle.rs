//! Connection lifecycle tests over real Unix sockets.
//!
//! These tests force graceful shutdown, drain, connection loss detection,
//! max_connections enforcement, and multi-client independence.

use std::time::Duration;

use super::harness::*;

/// Client sends GOODBYE → server drains → both sides Closed.
#[tokio::test]
async fn graceful_shutdown() {
    let mut f = connected_pair().await;

    f.send_request(b"pre-shutdown", TEST_TIMEOUT).await
        .expect("pre-shutdown request must succeed");

    {
        let requests = f.router.requests.lock();
        assert_eq!(requests.len(), 1);
    }

    f.take_client().shutdown().await;

    tokio::time::sleep(Duration::from_millis(1_000)).await;

    let changes = f.router.state_changes.lock();
    assert!(
        !changes.is_empty(),
        "server must report connection state change on shutdown, got none"
    );
    let last = changes.last().unwrap();
    assert!(
        last.new_state.contains("Closed"),
        "graceful shutdown must produce Closed state, got old={:?} new={:?}",
        last.old_state, last.new_state,
    );
}

/// When the client process disappears (drop without shutdown),
/// the server detects the broken pipe and surfaces ConnectionLost.
#[tokio::test]
async fn connection_lost_on_client_drop() {
    let mut f = connected_pair().await;

    f.send_request(b"before-drop", TEST_TIMEOUT).await
        .expect("request must succeed before drop");

    // Drop only the client — server stays alive to detect the disconnect
    drop(f.take_client());

    tokio::time::sleep(Duration::from_millis(1_000)).await;

    let changes = f.router.state_changes.lock();
    assert!(
        !changes.is_empty(),
        "server must detect connection loss and report state change, got none"
    );
    let last = changes.last().unwrap();
    assert!(
        last.new_state.contains("ConnectionLost") || last.new_state.contains("Closed"),
        "crash must produce ConnectionLost or Closed, got old={:?} new={:?}",
        last.old_state, last.new_state,
    );
}

/// Multiple clients connect simultaneously — all get independent sessions.
#[tokio::test]
async fn multiple_clients_independent() {
    let tempdir = tempfile::tempdir().unwrap();
    let socket_path = tempdir.path().join("multi.sock");

    let server_keypair = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_keypair.public.clone().try_into().unwrap();

    let server = IpcFixture::bind_server(
        &socket_path,
        server_keypair,
        IpcFixtureConfig::for_test(),
    ).await;

    let mut clients = Vec::new();
    for _ in 0..3 {
        let kp = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
        let client = rekindle_transport_ipc::v3::client::IpcClient::connect(
            &socket_path, &server_pub, &kp,
            IpcFixtureConfig::for_test().to_session_config(), handshake_config(),
        ).await.expect("client connect must succeed");
        clients.push(client);
    }

    for (i, client) in clients.iter().enumerate() {
        client.send_request(&[i as u8; 16], TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("client {i} request failed: {e:?}"));
    }

    let requests = server.router.requests.lock();
    assert_eq!(requests.len(), 3, "all 3 client requests must arrive");

    let payloads: Vec<Vec<u8>> = requests.iter().map(|r| r.payload.clone()).collect();
    assert!(payloads.contains(&vec![0u8; 16]));
    assert!(payloads.contains(&vec![1u8; 16]));
    assert!(payloads.contains(&vec![2u8; 16]));
    drop(requests);

    for client in clients {
        client.shutdown().await;
    }
}

/// The server enforces max_connections.
#[tokio::test]
async fn max_connections_enforced() {
    let tempdir = tempfile::tempdir().unwrap();
    let socket_path = tempdir.path().join("maxconn.sock");

    let server_keypair = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_keypair.public.clone().try_into().unwrap();

    let config = IpcFixtureConfig {
        max_connections: Some(2),
        ..IpcFixtureConfig::for_test()
    };

    let server = IpcFixture::bind_server(
        &socket_path,
        server_keypair,
        config.clone(),
    ).await;

    let session_config = config.to_session_config();

    let kp1 = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
    let kp2 = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
    let client1 = rekindle_transport_ipc::v3::client::IpcClient::connect(
        &socket_path, &server_pub, &kp1, session_config.clone(), handshake_config(),
    ).await.expect("client 1 must connect");
    let client2 = rekindle_transport_ipc::v3::client::IpcClient::connect(
        &socket_path, &server_pub, &kp2, session_config.clone(), handshake_config(),
    ).await.expect("client 2 must connect");

    client1.send_request(b"c1", Duration::from_secs(2)).await.expect("c1 request");
    client2.send_request(b"c2", Duration::from_secs(2)).await.expect("c2 request");

    let kp3 = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
    let connect_3 = tokio::time::timeout(
        Duration::from_millis(500),
        rekindle_transport_ipc::v3::client::IpcClient::connect(
            &socket_path, &server_pub, &kp3, session_config.clone(), handshake_config(),
        ),
    ).await;

    assert!(
        connect_3.is_err(),
        "3rd client must be blocked when max_connections=2"
    );

    client1.shutdown().await;
    tokio::time::sleep(Duration::from_millis(1_000)).await;

    let client3 = rekindle_transport_ipc::v3::client::IpcClient::connect(
        &socket_path, &server_pub, &kp3, session_config, handshake_config(),
    ).await.expect("client 3 must connect after slot freed");
    client3.send_request(b"c3", Duration::from_secs(2)).await.expect("c3 request");

    let requests = server.router.requests.lock();
    assert_eq!(requests.len(), 3, "c1, c2, c3 requests must all arrive");
    let payloads: Vec<&[u8]> = requests.iter().map(|r| r.payload.as_slice()).collect();
    assert!(payloads.contains(&b"c1".as_slice()), "c1 payload missing");
    assert!(payloads.contains(&b"c2".as_slice()), "c2 payload missing");
    assert!(payloads.contains(&b"c3".as_slice()), "c3 payload missing");
    drop(requests);

    client2.shutdown().await;
    client3.shutdown().await;
}
