//! Bulk transfer pipeline tests — forces the complete
//! STREAM_OPEN → rayon encrypt → STREAM_PAYLOAD → reassembler →
//! content hash verify → STREAM_FIN → STREAM_ACK cycle over a real socket.

use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn small_bulk_single_chunk() {
    let f = connected_pair().await;

    let payload = vec![0xAA; 1024];
    let result = f.send_bulk(0, &payload, TEST_TIMEOUT).await;

    assert!(result.is_ok(), "small bulk must succeed: {result:?}");
    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, 1024);

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, 1024);
}

#[tokio::test]
async fn multi_chunk_bulk_16mib() {
    let f = connected_pair().await;

    let payload = vec![0xBB; 16 * 1024 * 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(30)).await;

    assert!(result.is_ok(), "16 MiB bulk must succeed: {result:?}");
    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);
    assert!(delivered.chunks >= 2, "16 MiB must produce multiple chunks");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
}

#[tokio::test]
async fn multi_chunk_bulk_48mib() {
    let f = connected_pair().await;

    let payload = vec![0xFE; 48 * 1024 * 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(60)).await;

    assert!(result.is_ok(), "48 MiB bulk must succeed: {result:?}");
    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);
    assert!(delivered.chunks >= 3, "48 MiB must produce at least 3 chunks");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
}

#[tokio::test]
async fn concurrent_bulk_on_different_streams() {
    let f = connected_pair().await;

    let payload_a = vec![0xAA; 2 * 1024 * 1024];
    let payload_b = vec![0xBB; 2 * 1024 * 1024];

    let (ra, rb) = tokio::join!(
        f.send_bulk(0, &payload_a, Duration::from_secs(15)),
        f.send_bulk(1, &payload_b, Duration::from_secs(15)),
    );

    assert!(ra.is_ok(), "stream 0 must succeed: {ra:?}");
    assert!(rb.is_ok(), "stream 1 must succeed: {rb:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 2, "both streams must complete");
}

#[tokio::test]
async fn bulk_then_request_still_works() {
    let f = connected_pair().await;

    let payload = vec![0xCC; 1024 * 1024];
    f.send_bulk(0, &payload, Duration::from_secs(10)).await
        .expect("bulk must succeed");

    f.send_request(b"post-bulk-ping", TEST_TIMEOUT).await
        .expect("request after bulk must succeed");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].payload, b"post-bulk-ping".as_slice());
}

#[tokio::test]
async fn bulk_recv_delivers_data_to_client() {
    let f = connected_pair().await;

    // Server sends bulk to client via the connection handle's BulkSender
    let bulk_sender = f.bulk_sender();
    let payload = vec![0xDD; 64 * 1024];
    let clearance = f.agreed_clearance();

    let chunk_count = bulk_sender.send(0, &payload, clearance).await
        .expect("server bulk send must succeed");
    assert!(chunk_count >= 1);

    // Client receives the bulk data
    let received = tokio::time::timeout(
        Duration::from_secs(10),
        f.recv_bulk(),
    ).await.expect("recv_bulk must not timeout");

    assert!(received.is_some(), "client must receive bulk data");
    let (stream_id, data) = received.unwrap();
    assert_eq!(stream_id, 0);
    assert_eq!(data.len(), payload.len());
    assert_eq!(data, payload);
}

#[tokio::test]
async fn bulk_cancel_and_reuse_stream_id() {
    let f = connected_pair().await;

    let payload = vec![0xDD; 2048];
    f.send_bulk(5, &payload, TEST_TIMEOUT).await
        .expect("initial transfer must succeed");

    f.cancel_bulk(5).await;

    let payload2 = vec![0xEE; 1024];
    let result = f.send_bulk(5, &payload2, TEST_TIMEOUT).await;
    assert!(result.is_ok(), "recycled stream_id must succeed: {result:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sequential_bulk_transfers_10x() {
    let f = connected_pair().await;

    for i in 0..10u8 {
        let payload = vec![i; 256 * 1024];
        f.send_bulk(i, &payload, Duration::from_secs(10)).await
            .unwrap_or_else(|e| panic!("transfer {i} failed: {e:?}"));
    }

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 10, "all 10 transfers must complete");
}
