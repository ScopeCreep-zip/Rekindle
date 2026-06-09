//! Stream tests over real Unix sockets — bulk transfer lifecycle.

use std::time::{Duration, Instant};

use super::harness::*;

#[tokio::test]
async fn stream_open_payload_fin_ack_full_lifecycle() {
    let f = connected_pair().await;

    let payload = vec![0xAB; 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(10)).await;

    assert!(result.is_ok(), "send_bulk must succeed: {result:?}");

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, 1024);
    assert_eq!(delivered.chunks, 1);

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1, "server must fire on_bulk_complete");
    assert_eq!(completes[0].total_bytes, 1024);
    assert_eq!(completes[0].total_chunks, 1);
}

#[tokio::test]
async fn stream_content_hash_verified() {
    let f = connected_pair().await;

    let payload = b"verified content";
    let result = f.send_bulk(0, payload, TEST_TIMEOUT).await;
    assert!(result.is_ok(), "valid hash must succeed: {result:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
}

#[tokio::test]
async fn single_chunk_above_bulk_threshold() {
    let f = connected_pair().await;

    let payload = vec![0xEF; 100 * 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(10)).await;

    assert!(result.is_ok(), "single chunk above bulk threshold must succeed: {result:?}");

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);
    assert_eq!(delivered.chunks, 1);

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
}

#[tokio::test]
async fn single_512_bytes_stream_0() {
    let f = connected_pair().await;

    let payload = vec![0xAA; 512];
    let result = f.send_bulk(0, &payload, TEST_TIMEOUT).await;
    assert!(result.is_ok(), "512 bytes on stream 0 must succeed: {result:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, 512);
}

#[tokio::test]
async fn single_512_bytes_stream_1() {
    let f = connected_pair().await;

    let payload = vec![0xBB; 512];
    let result = f.send_bulk(1, &payload, TEST_TIMEOUT).await;
    assert!(result.is_ok(), "512 bytes on stream 1 must succeed: {result:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, 512);
}

#[tokio::test]
async fn sequential_streams_on_different_ids() {
    let f = connected_pair().await;

    let payload_0 = vec![0xAA; 512];
    let result_0 = f.send_bulk(0, &payload_0, TEST_TIMEOUT).await;
    assert!(result_0.is_ok(), "stream 0 sequential must succeed: {result_0:?}");

    let payload_1 = vec![0xBB; 768];
    let result_1 = f.send_bulk(1, &payload_1, TEST_TIMEOUT).await;
    assert!(result_1.is_ok(), "stream 1 sequential must succeed: {result_1:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 2, "both transfers must complete");
    let mut bytes: Vec<u64> = completes.iter().map(|c| c.total_bytes).collect();
    bytes.sort();
    assert_eq!(bytes, vec![512, 768]);
}

#[tokio::test]
async fn sequential_reuse_same_stream_id() {
    let f = connected_pair().await;

    let payload_0 = vec![0xCC; 256];
    let result_0 = f.send_bulk(0, &payload_0, TEST_TIMEOUT).await;
    assert!(result_0.is_ok(), "first transfer must succeed: {result_0:?}");

    let payload_1 = vec![0xDD; 384];
    let result_1 = f.send_bulk(0, &payload_1, TEST_TIMEOUT).await;
    assert!(result_1.is_ok(), "second transfer on same stream_id must succeed: {result_1:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 2, "both transfers must complete");
    let mut bytes: Vec<u64> = completes.iter().map(|c| c.total_bytes).collect();
    bytes.sort();
    assert_eq!(bytes, vec![256, 384]);
}

#[tokio::test]
async fn concurrent_streams_independent() {
    let f = connected_pair().await;

    let payload_0 = vec![0xAA; 512];
    let payload_1 = vec![0xBB; 768];

    let (r0, r1) = tokio::join!(
        f.send_bulk(0, &payload_0, TEST_TIMEOUT),
        f.send_bulk(1, &payload_1, TEST_TIMEOUT),
    );

    assert!(r0.is_ok(), "stream 0 must succeed: {r0:?}");
    assert!(r1.is_ok(), "stream 1 must succeed: {r1:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 2, "both concurrent transfers must complete");
    let mut bytes: Vec<u64> = completes.iter().map(|c| c.total_bytes).collect();
    bytes.sort();
    assert_eq!(bytes, vec![512, 768]);
}

#[tokio::test]
async fn stream_large_multi_chunk_transfer() {
    let f = connected_pair().await;

    let payload = vec![0xCD; 48 * 1024 * 1024];
    let start = Instant::now();
    let result = f.send_bulk(0, &payload, Duration::from_secs(30)).await;
    let elapsed = start.elapsed();

    if result.is_err() {
        let completes = f.router.bulk_completes.lock();
        let complete_count = completes.len();
        drop(completes);

        panic!(
            "large send_bulk failed after {elapsed:.2?}: {:?}\n\
             bulk_completes fired: {complete_count}\n\
             client phase: {:?}",
            result.unwrap_err(),
            f.phase(),
        );
    }

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);
    assert!(delivered.chunks >= 3, "48 MiB must produce at least 3 chunks");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);

    assert!(
        elapsed < Duration::from_secs(15),
        "48 MiB transfer took {elapsed:.2?} — expected under 15s even in debug"
    );
}

#[tokio::test]
async fn parallel_encoding_does_not_block_heartbeat() {
    let f = connected_pair().await;

    let payload = vec![0xEE; 48 * 1024 * 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(30)).await;

    assert!(result.is_ok(), "bulk must succeed without heartbeat death: {result:?}");

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
    drop(completes);

    tokio::time::sleep(Duration::from_millis(1_500)).await;

    assert!(
        !f.phase().is_terminal(),
        "connection must survive heartbeat during large transfer"
    );
}

#[tokio::test]
async fn concurrent_large_streams_saturate_pool() {
    let f = connected_pair().await;

    let payload_0 = vec![0xAA; 48 * 1024 * 1024];
    let payload_1 = vec![0xBB; 48 * 1024 * 1024];
    let payload_2 = vec![0xCC; 48 * 1024 * 1024];
    let payload_3 = vec![0xDD; 48 * 1024 * 1024];

    let start = Instant::now();
    let (r0, r1, r2, r3) = tokio::join!(
        f.send_bulk(0, &payload_0, Duration::from_secs(30)),
        f.send_bulk(1, &payload_1, Duration::from_secs(30)),
        f.send_bulk(2, &payload_2, Duration::from_secs(30)),
        f.send_bulk(3, &payload_3, Duration::from_secs(30)),
    );
    let elapsed = start.elapsed();

    if r0.is_err() || r1.is_err() || r2.is_err() || r3.is_err() {
        let completes = f.router.bulk_completes.lock().len();
        panic!(
            "concurrent 4-stream transfer failed after {elapsed:.2?}:\n\
             stream 0: {r0:?}\nstream 1: {r1:?}\nstream 2: {r2:?}\nstream 3: {r3:?}\n\
             bulk_completes fired: {completes}\nclient phase: {:?}",
            f.phase(),
        );
    }

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 4, "all 4 streams must complete");
    for c in completes.iter() {
        assert_eq!(c.total_bytes, 48 * 1024 * 1024, "stream {} byte count", c.stream_id);
    }

    assert!(elapsed < Duration::from_secs(60), "4 × 48 MiB took {elapsed:.2?}");
}

#[tokio::test]
async fn small_frame_inline_encoding_latency() {
    let f = connected_pair().await;

    let start = Instant::now();
    for i in 0..1000u16 {
        f.send_request(&i.to_le_bytes(), TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }
    let elapsed = start.elapsed();

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1000, "all 1000 requests must arrive");

    assert!(elapsed < Duration::from_secs(2), "1000 small requests took {elapsed:.2?}");
}
