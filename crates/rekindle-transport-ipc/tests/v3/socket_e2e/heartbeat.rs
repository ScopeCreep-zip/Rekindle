//! Heartbeat tests over real Unix sockets.

use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn heartbeat_keeps_connection_alive() {
    let f = connected_pair().await;

    tokio::time::sleep(Duration::from_millis(2_500)).await;

    assert!(
        !f.phase().is_terminal(),
        "connection must survive 2 heartbeat cycles"
    );

    let result = f.send_request(b"still-alive", TEST_TIMEOUT).await;
    assert!(result.is_ok(), "must be able to send after heartbeat: {result:?}");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].payload, b"still-alive".as_slice());
}

#[tokio::test]
async fn heartbeat_timeout_kills_connection() {
    let f = connected_pair().await;

    f.cancel.cancel();

    tokio::time::sleep(Duration::from_millis(6_000)).await;

    assert!(
        f.phase().is_terminal(),
        "connection must be dead after heartbeat timeout, got {:?}",
        f.phase()
    );
}

#[tokio::test]
async fn single_missed_pong_does_not_kill() {
    let f = connected_pair().await;

    tokio::time::sleep(Duration::from_millis(3_000)).await;

    assert!(
        !f.phase().is_terminal(),
        "client must be alive after normal operation"
    );

    let result = f.send_request(b"post-heartbeat", TEST_TIMEOUT).await;
    assert!(result.is_ok(), "request after heartbeat cycles must succeed");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1, "server must receive the post-heartbeat request");
    assert_eq!(requests[0].payload, b"post-heartbeat".as_slice());
}
