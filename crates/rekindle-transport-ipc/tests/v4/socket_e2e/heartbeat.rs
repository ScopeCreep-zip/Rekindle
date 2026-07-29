//! Heartbeat tests — connection liveness detection over real sockets.

use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn heartbeat_keeps_connection_alive() {
    let f = connected_pair().await;

    // Wait for 2 heartbeat cycles (interval=1000ms, miss_limit=3)
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    assert!(!f.phase().is_terminal(), "connection must survive 2 heartbeat cycles");

    f.send_request(b"still-alive", TEST_TIMEOUT).await
        .expect("must be able to send after heartbeat");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].payload, b"still-alive".as_slice());
}

#[tokio::test]
async fn heartbeat_timeout_kills_connection() {
    let f = connected_pair().await;

    // Cancel the server — peer stops responding to PINGs
    f.cancel.cancel();

    // Wait for heartbeat timeout (miss_limit=3 × response_timeout=500ms + margin)
    tokio::time::sleep(Duration::from_millis(6_000)).await;

    assert!(
        f.phase().is_terminal(),
        "connection must be dead after heartbeat timeout, got {:?}",
        f.phase()
    );
}
