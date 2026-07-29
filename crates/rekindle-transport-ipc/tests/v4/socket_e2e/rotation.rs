//! Key rotation tests — proves post-rotation frames decrypt correctly
//! through the full io_uring read/write pipeline with epoch-tagged AEAD.

use std::time::Duration;

use super::harness::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rotation_preserves_bidirectional_communication() {
    init_tracing();
    let f = connected_pair().await;

    f.send_request(b"pre-rotation", TEST_TIMEOUT).await
        .expect("pre-rotation request must succeed");

    f.rotate_keys(TEST_TIMEOUT).await
        .expect("rotation must complete");

    f.send_request(b"post-rotation", TEST_TIMEOUT).await
        .expect("post-rotation request must succeed — new keys must match");

    let bulk_data = vec![0xCC; 64 * 1024];
    f.send_bulk(0, &bulk_data, Duration::from_secs(10)).await
        .expect("post-rotation bulk transfer must succeed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rotation_on_idle_session() {
    init_tracing();
    let f = connected_pair().await;

    f.rotate_keys(TEST_TIMEOUT).await
        .expect("rotation on idle session must succeed");

    f.send_request(b"alive-after-rotation", TEST_TIMEOUT).await
        .expect("session must be functional after rotation");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn double_rotation() {
    init_tracing();
    let f = connected_pair().await;

    f.rotate_keys(TEST_TIMEOUT).await.expect("first rotation");
    f.send_request(b"after-r1", TEST_TIMEOUT).await.expect("after first");

    f.rotate_keys(TEST_TIMEOUT).await.expect("second rotation");
    f.send_request(b"after-r2", TEST_TIMEOUT).await.expect("after second");

    let bulk = vec![0xDD; 64 * 1024];
    f.send_bulk(0, &bulk, Duration::from_secs(10)).await
        .expect("bulk after double rotation must succeed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rotation_20_cycles() {
    init_tracing();
    let f = connected_pair().await;

    for i in 0..20 {
        f.rotate_keys(TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("rotation {i} failed: {e:?}"));
        f.send_request(
            format!("alive-{i}").as_bytes(),
            TEST_TIMEOUT,
        ).await
            .unwrap_or_else(|e| panic!("request after rotation {i} failed: {e:?}"));
    }

    let bulk = vec![0xFF; 64 * 1024];
    f.send_bulk(0, &bulk, Duration::from_secs(10)).await
        .expect("bulk after 20 rotations must succeed");
}
