//! Pool integration tests over real Unix sockets.

use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn pool_slabs_return_after_transfer_completes() {
    let f = connected_pair().await;

    let payload = vec![0xAB; 1024];
    let result = f.send_bulk(0, &payload, TEST_TIMEOUT).await;
    assert!(result.is_ok(), "transfer must succeed: {result:?}");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, 1024);
}

#[tokio::test]
async fn concurrent_transfers_share_pool_without_corruption() {
    let f = connected_pair().await;

    let payloads: Vec<Vec<u8>> = (0..4u8).map(|i| vec![i; 2048]).collect();

    let (r0, r1, r2, r3) = tokio::join!(
        f.send_bulk(0, &payloads[0], TEST_TIMEOUT),
        f.send_bulk(1, &payloads[1], TEST_TIMEOUT),
        f.send_bulk(2, &payloads[2], TEST_TIMEOUT),
        f.send_bulk(3, &payloads[3], TEST_TIMEOUT),
    );

    assert!(r0.is_ok(), "stream 0: {r0:?}");
    assert!(r1.is_ok(), "stream 1: {r1:?}");
    assert!(r2.is_ok(), "stream 2: {r2:?}");
    assert!(r3.is_ok(), "stream 3: {r3:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 4, "all 4 transfers must complete");
    for c in completes.iter() {
        assert_eq!(c.total_bytes, 2048, "stream {} byte count", c.stream_id);
    }
}

#[tokio::test]
async fn pool_backpressure_keeps_connection_alive() {
    let f = connected_pair().await;

    let payload = vec![0xEE; 48 * 1024 * 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(30)).await;
    assert!(result.is_ok(), "transfer must complete despite pool backpressure: {result:?}");

    assert!(
        !f.phase().is_terminal(),
        "connection must survive pool backpressure"
    );

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
}
