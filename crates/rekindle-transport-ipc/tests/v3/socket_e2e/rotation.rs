//! Key rotation E2E test — proves post-rotation frames decrypt correctly.

use std::time::Duration;

use super::harness::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rotation_preserves_bidirectional_communication() {
    init_tracing();
    let f = connected_pair().await;

    f.send_request(b"pre-rotation-ping", TEST_TIMEOUT).await
        .expect("pre-rotation request must succeed");

    f.rotate_keys(TEST_TIMEOUT).await
        .expect("rotation must complete successfully");

    f.send_request(b"post-rotation-ping", TEST_TIMEOUT).await
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
async fn rotation_twice_preserves_communication() {
    init_tracing();
    let f = connected_pair().await;

    f.rotate_keys(TEST_TIMEOUT).await
        .expect("first rotation must succeed");
    f.send_request(b"after-rotation-1", TEST_TIMEOUT).await
        .expect("request after first rotation must succeed");

    f.rotate_keys(TEST_TIMEOUT).await
        .expect("second rotation must succeed");
    f.send_request(b"after-rotation-2", TEST_TIMEOUT).await
        .expect("request after second rotation must succeed");

    let bulk_data = vec![0xDD; 64 * 1024];
    f.send_bulk(0, &bulk_data, Duration::from_secs(10)).await
        .expect("bulk after double rotation must succeed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rotation_sustained_20_cycles() {
    init_tracing();
    let f = connected_pair().await;

    for i in 0..20 {
        f.rotate_keys(TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("rotation {i} failed: {e:?}"));
        f.send_request(
            format!("alive-after-rotation-{i}").as_bytes(),
            TEST_TIMEOUT,
        ).await
            .unwrap_or_else(|e| panic!("request after rotation {i} failed: {e:?}"));
    }

    let bulk_data = vec![0xFF; 64 * 1024];
    f.send_bulk(0, &bulk_data, Duration::from_secs(10)).await
        .expect("bulk after 20 rotations must succeed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rotation_retires_old_keys_after_window() {
    init_tracing();
    let f = connected_pair().await;

    f.rotate_keys(TEST_TIMEOUT).await
        .expect("rotation must succeed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    f.send_request(b"after-retirement-window", TEST_TIMEOUT).await
        .expect("session must work after retirement window");

    let bulk_data = vec![0xEE; 64 * 1024];
    f.send_bulk(0, &bulk_data, Duration::from_secs(10)).await
        .expect("bulk after retirement window must succeed");
}
