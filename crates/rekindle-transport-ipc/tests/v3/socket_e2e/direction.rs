//! Key direction tests — prove d2l/l2d are correct by sending
//! frames in both directions and verifying decryption succeeds.

use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn client_to_server_request_decrypts() {
    let f = connected_pair().await;

    let payload = b"hello from client";
    let result = f.send_request(payload, TEST_TIMEOUT).await;

    assert!(result.is_ok(), "client→server failed: {result:?}");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1, "server must receive exactly 1 request");
    assert_eq!(requests[0].payload, payload, "payload must match");
}

#[tokio::test]
async fn server_to_client_notify_decrypts() {
    let f = connected_pair().await;

    // Wait for one heartbeat cycle (config is 1s interval).
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    assert!(
        !f.phase().is_terminal(),
        "connection must survive a heartbeat cycle — l2d direction is wrong if terminal"
    );

    let result = f.send_request(b"post-heartbeat", TEST_TIMEOUT).await;
    assert!(result.is_ok(), "request after heartbeat must succeed: {result:?}");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1, "server must receive the post-heartbeat request");
}

#[tokio::test]
async fn bidirectional_simultaneous() {
    let f = connected_pair().await;

    for i in 0..10u8 {
        let payload = vec![i; 64];
        let result = f.send_request(&payload, TEST_TIMEOUT).await;
        assert!(result.is_ok(), "request {i} failed: {result:?}");
    }

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 10, "server must receive all 10 requests");
    for (i, req) in requests.iter().enumerate() {
        assert_eq!(req.payload, vec![i as u8; 64], "request {i} payload mismatch");
    }
}

/// Server→client bulk transfer — proves the l2d bulk path works.
#[tokio::test]
async fn server_to_client_bulk_transfer() {
    let f = connected_pair().await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let conn_handle = f.conn_handle.lock().clone()
        .expect("server connection handle must be populated after connect");

    let bulk_data = vec![0xAB; 64 * 1024];
    conn_handle.bulk_sender.send(0, &bulk_data, rekindle_transport_ipc::v3::wire::clearance::Clearance::Internal)
        .await
        .expect("server→client bulk send must succeed");

    let received = tokio::time::timeout(
        TEST_TIMEOUT,
        f.recv_bulk(),
    ).await
        .expect("recv_bulk must not timeout")
        .expect("recv_bulk must return Some");

    assert_eq!(received.0, 0, "stream_id must be 0");
    assert_eq!(received.1.len(), bulk_data.len(), "received length must match sent length");
    assert_eq!(received.1, bulk_data, "received payload must match sent payload");
}

/// Repeated server→client bulk on the same stream_id.
/// Forces the stream registry to handle open→close→reopen cycles.
/// If the registry lacks direction awareness or fails to close before
/// reopen, this test catches it.
#[tokio::test]
async fn server_to_client_repeated_same_stream_id() {
    let f = connected_pair().await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let conn_handle = f.conn_handle.lock().clone()
        .expect("server connection handle must be populated after connect");

    let data = vec![0xCD; 1024];
    for i in 0..10u32 {
        conn_handle.bulk_sender.send(0, &data, rekindle_transport_ipc::v3::wire::clearance::Clearance::Internal)
            .await
            .unwrap_or_else(|e| panic!("server→client transfer {i} on stream_id 0 failed: {e:?}"));

        let received = tokio::time::timeout(TEST_TIMEOUT, f.recv_bulk())
            .await
            .unwrap_or_else(|_| panic!("recv_bulk timed out on transfer {i}"))
            .unwrap_or_else(|| panic!("recv_bulk returned None on transfer {i}"));

        assert_eq!(received.0, 0, "transfer {i}: stream_id must be 0");
        assert_eq!(received.1.len(), data.len(), "transfer {i}: length mismatch");
    }
}

/// Simultaneous bidirectional bulk — client→server and server→client
/// on the SAME stream_id at the same time. Forces the registry to
/// handle both directions independently.
#[tokio::test]
async fn bidirectional_bulk_same_stream_id() {
    let f = connected_pair().await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let conn_handle = f.conn_handle.lock().clone()
        .expect("server connection handle must be populated after connect");

    let client_data = vec![0xAA; 2048];
    let server_data = vec![0xBB; 2048];

    // Client→server and server→client on stream_id 0 concurrently
    let client_fut = f.send_bulk(0, &client_data, TEST_TIMEOUT);
    let server_fut = conn_handle.bulk_sender.send(0, &server_data, rekindle_transport_ipc::v3::wire::clearance::Clearance::Internal);

    let (client_result, server_result) = tokio::join!(client_fut, server_fut);
    client_result.expect("client→server bulk must succeed");
    server_result.expect("server→client bulk must succeed");

    // Client receives server's bulk
    let received = tokio::time::timeout(TEST_TIMEOUT, f.recv_bulk())
        .await
        .expect("recv_bulk must not timeout")
        .expect("recv_bulk must return Some");
    assert_eq!(received.1.len(), server_data.len());
}

/// Repeated client→server bulk on the same stream_id.
/// Mirrors server_to_client_repeated_same_stream_id but from client side.
#[tokio::test]
async fn client_to_server_repeated_same_stream_id() {
    let f = connected_pair().await;

    let data = vec![0xEF; 1024];
    for i in 0..10u32 {
        f.send_bulk(0, &data, TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("client→server transfer {i} on stream_id 0 failed: {e:?}"));
    }

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 10, "all 10 client→server transfers must complete");
    for (i, c) in completes.iter().enumerate() {
        assert_eq!(c.total_bytes, 1024, "transfer {i} byte count");
    }
}
