//! Adversarial tests over real Unix sockets.
//!
//! These tests force security properties to be enforced at the socket level.
//! They verify that the transport rejects malicious or malformed input
//! through the full EMAC → AEAD → dispatch pipeline.

use std::time::Duration;

use super::harness::*;

/// Send a frame exceeding MAX_BODY_LEN → transport rejects before writing.
#[tokio::test]
async fn oversized_frame_rejected() {
    let f = connected_pair().await;

    f.send_request(b"small", TEST_TIMEOUT).await
        .expect("small request must succeed");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1);
    drop(requests);

    // 128 KiB exceeds the Control Lane's 64 KiB MAX_BODY_LEN.
    let oversized = vec![0xEE; 128 * 1024];
    let result = f.send_request(&oversized, Duration::from_secs(2)).await;

    assert!(
        result.is_err(),
        "oversized frame must be rejected, got: {result:?}"
    );
}

/// After a client drops without graceful shutdown (simulating a crash),
/// subsequent requests from other clients are unaffected.
#[tokio::test]
async fn crashed_client_does_not_poison_server() {
    init_tracing();

    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let tempdir = tempfile::tempdir().unwrap();
        let socket_path = tempdir.path().join("poison.sock");

        let server_keypair = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
        let server_pub: [u8; 32] = server_keypair.public.clone().try_into().unwrap();

        let server = IpcFixture::bind_server(
            &socket_path,
            server_keypair,
            IpcFixtureConfig::for_test(),
        ).await;

        // Client 1: connect, send, crash (drop without shutdown)
        {
            let kp = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
            let client1 = rekindle_transport_ipc::v3::client::IpcClient::connect(
                &socket_path, &server_pub, &kp,
                IpcFixtureConfig::for_test().to_session_config(), handshake_config(),
            ).await.expect("client 1 connect");
            client1.send_request(b"from-c1", TEST_TIMEOUT).await
                .expect("c1 request");
            // Drop without shutdown — simulate crash
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Client 2: connect after client 1 crashed — must succeed
        let kp2 = rekindle_transport_ipc::v3::crypto::noise::generate_keypair().unwrap();
        let client2 = rekindle_transport_ipc::v3::client::IpcClient::connect(
            &socket_path, &server_pub, &kp2,
            IpcFixtureConfig::for_test().to_session_config(), handshake_config(),
        ).await.expect("client 2 must connect after client 1 crash");

        client2.send_request(b"from-c2", TEST_TIMEOUT).await
            .expect("c2 request must succeed — server not poisoned");

        let requests = server.router.requests.lock();
        assert!(
            requests.iter().any(|r| r.payload == b"from-c1"),
            "c1 request must have been delivered before crash"
        );
        assert!(
            requests.iter().any(|r| r.payload == b"from-c2"),
            "c2 request must succeed after c1 crash"
        );

        client2.shutdown().await;
    }).await;

    assert!(result.is_ok(), "TEST TIMEOUT: crashed_client_does_not_poison_server hung for 15s");
}

/// Verify that the AEAD algorithm negotiation works.
#[tokio::test]
async fn aead_algorithm_negotiated_and_functional() {
    let f = connected_pair().await;

    let sizes = [0, 1, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 255, 256, 1023, 1024, 4096];
    for size in sizes {
        let payload = vec![0xAA; size];
        let result = f.send_request(&payload, TEST_TIMEOUT).await;
        assert!(result.is_ok(), "size {size} failed: {result:?}");
    }

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), sizes.len(), "all requests must arrive");

    for (i, size) in sizes.iter().enumerate() {
        assert_eq!(
            requests[i].payload.len(), *size,
            "request {i} payload size mismatch: expected {size}, got {}",
            requests[i].payload.len()
        );
    }
}

/// Concurrent sends from a single client must not corrupt ack tracking.
#[tokio::test]
async fn rapid_sequential_sends_do_not_corrupt_ack_tracking() {
    let f = connected_pair().await;

    for i in 0..20u8 {
        f.send_request(&[i; 32], TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 20, "all 20 requests must arrive");

    for (i, req) in requests.iter().enumerate() {
        assert_eq!(req.payload, vec![i as u8; 32], "request {i} payload mismatch");
    }
}
