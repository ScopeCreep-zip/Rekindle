//! Audit chain tests over real Unix sockets.

use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn audit_checkpoint_emitted_after_frame_cadence() {
    let f = connected_pair().await;

    for i in 0..50u16 {
        f.send_request(&i.to_le_bytes(), Duration::from_secs(30)).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 50, "all 50 frames must arrive without chain divergence");
    drop(requests);

    assert!(!f.phase().is_terminal(), "connection must survive checkpoint verification");
}

#[tokio::test]
async fn outbound_audit_chain_advanced_on_send() {
    let f = connected_pair().await;

    for i in 0..10u8 {
        f.send_request(&[i; 32], TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }

    {
        let requests = f.router.requests.lock();
        assert_eq!(requests.len(), 10);
    }

    tokio::time::sleep(Duration::from_millis(1_500)).await;

    assert!(
        !f.phase().is_terminal(),
        "connection must survive heartbeat — outbound audit chain broken if dead"
    );
}

#[tokio::test]
async fn audit_chain_uses_real_hashes_not_zeros() {
    let config = IpcFixtureConfig {
        ..IpcFixtureConfig::for_test()
    };
    // TODO: expose checkpoint_config through IpcFixtureConfig if needed
    // For now use default which has max_frames high enough
    let f = connected_pair_with_config(config).await;

    for i in 0..10u8 {
        f.send_request(&[i; 64], TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        !f.phase().is_terminal(),
        "connection must survive checkpoint — audit chain uses zero hashes if dead"
    );

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 10, "all requests must arrive");
}

#[tokio::test]
async fn no_spurious_gaps_under_normal_delivery() {
    let f = connected_pair().await;

    for i in 0..499u16 {
        f.send_notify(&i.to_le_bytes()).await
            .unwrap_or_else(|e| panic!("notify {i} failed: {e:?}"));
    }

    f.send_request(&499u16.to_le_bytes(), Duration::from_secs(10)).await
        .expect("final sync request must succeed");

    let notifications = f.router.notifications.lock();
    let requests = f.router.requests.lock();
    assert_eq!(
        notifications.len() + requests.len(), 500,
        "all 500 must arrive without spurious gap detection"
    );
}
