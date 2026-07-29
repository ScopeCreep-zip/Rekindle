//! Datagram tests over real Unix sockets — request/reply, notify.
//! Proves the control plane works end-to-end through io_uring read/write
//! tasks, AEAD encryption, lane demux, handler dispatch, and router delivery.

use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn request_delivered_to_router_with_correct_payload() {
    let f = connected_pair().await;

    let payload = b"daemon:get_status";
    f.send_request(payload, TEST_TIMEOUT).await
        .expect("send_request must succeed");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].payload, payload.as_slice());
}

#[tokio::test]
async fn request_reply_returns_payload() {
    let f = connected_pair().await;

    let result = f.request_reply(b"echo-me", TEST_TIMEOUT).await
        .expect("request_reply must succeed");

    // ReplyRouter echoes the request payload back
    assert_eq!(result.payload, b"echo-me".as_slice());
}

#[tokio::test]
async fn notify_delivered_to_router() {
    let f = connected_pair().await;

    f.send_notify(b"vault-unlocked").await
        .expect("send_notify must succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let notifications = f.router.notifications.lock();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].payload, b"vault-unlocked".as_slice());
}

#[tokio::test]
async fn fifty_requests_all_delivered_in_order() {
    let f = connected_pair().await;

    for i in 0..50u8 {
        f.send_request(&[i; 32], TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 50);
    for (i, req) in requests.iter().enumerate() {
        assert_eq!(req.payload, vec![i as u8; 32], "request {i} payload mismatch");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sustained_request_reply_1000_multi_thread() {
    let f = connected_pair().await;

    for i in 0..1000u32 {
        let payload = i.to_le_bytes();
        f.send_request(&payload, TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 1000);
}
